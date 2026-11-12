use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

use bitcoin::{
    Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxMerkleNode, TxOut, Txid,
    Witness,
    absolute::LockTime,
    block::{Header, Version as BlockVersion},
    blockdata::constants::genesis_block,
    consensus::encode::serialize,
    hashes::{Hash as _, sha256, sha256d},
    opcodes::all::OP_PUSHNUM_1,
    pow::CompactTarget,
    transaction::Version as TxVersion,
};
use parking_lot::Mutex;

/// Timestamp of block 1; every later block is ten minutes after its parent
const FIRST_BLOCK_TIME: u32 = 1_700_000_000;
const BLOCK_INTERVAL_SECS: u32 = 600;

/// Regtest difficulty; nothing in the scan stack checks proof of work
const EASY_BITS: u32 = 0x207f_ffff;

const FUNDING_FEE: Amount = Amount::from_sat(1_000);

/// Bitcoin Core's `-incrementalrelayfee` default before 29.1, in sat/vB
///
/// Later releases lower it to 0.1 sat/vB, but many nodes still run the older
/// default, so a wallet's replacement has to clear this one to relay everywhere
const INCREMENTAL_RELAY_SAT_PER_VB: u64 = 1;

/// Core's `MAX_REPLACEMENT_CANDIDATES`: a replacement may evict at most this many transactions
const MAX_REPLACEMENT_CANDIDATES: usize = 100;

/// Core's `-bytespersigop` default, the virtual bytes each signature operation counts as
const BYTES_PER_SIGOP: u64 = 20;

/// Where a transaction is in the fake chain
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TxLocation {
    Mempool,
    Confirmed { height: u32 },
}

/// A transaction together with its location in the fake chain
#[derive(Debug, Clone)]
pub struct ChainTx {
    pub tx: Transaction,
    /// Whether the transaction is in the mempool or which block holds it
    pub location: TxLocation,
    /// Position of the transaction inside its block, only set once confirmed
    pub position: Option<usize>,
}

/// One entry of a script's history, in the shape both node protocols report it
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct HistoryEntry {
    pub txid: Txid,
    /// Whether the transaction is in the mempool or which block holds it
    pub location: TxLocation,
}

/// Net satoshis a script gains, split by whether the transactions are confirmed
///
/// A pending spend of confirmed funds makes `unconfirmed` negative, as in electrs
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct Balance {
    pub confirmed: i64,
    pub unconfirmed: i64,
}

/// Why the node refused a broadcast
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BroadcastError {
    /// The transaction is already in a block
    AlreadyConfirmed,
    /// An input spends an output the chain has never seen
    MissingInput(OutPoint),
    /// An input spends an output that a confirmed transaction already spent
    SpentInput(OutPoint),
    /// The outputs add up to more than the inputs, or to more than an amount can hold
    OutputsExceedInputs,
    /// The transaction double-spends pending ones without meeting the replacement rules
    Replacement(ReplacementError),
    /// Input `index` does not satisfy the script of the output it spends, such as a
    /// missing or invalid signature
    InvalidScript { index: usize, error: bitcoinconsensus::Error },
}

/// Messages start with bitcoind's reject reasons, so a client that reports them
/// shows what a real node would send
impl fmt::Display for BroadcastError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyConfirmed => f.write_str("Transaction already in block chain"),
            Self::MissingInput(outpoint) | Self::SpentInput(outpoint) => {
                write!(f, "bad-txns-inputs-missingorspent ({outpoint})")
            }

            Self::OutputsExceedInputs => f.write_str("bad-txns-in-belowout"),
            Self::Replacement(error) => error.fmt(f),

            // ERR_SCRIPT is libbitcoinconsensus's "no error" value, set when the script itself fails
            Self::InvalidScript { index, error: bitcoinconsensus::Error::ERR_SCRIPT } => {
                write!(f, "mandatory-script-verify-flag-failed (input {index})")
            }

            Self::InvalidScript { index, error } => {
                write!(f, "mandatory-script-verify-flag-failed (input {index}: {error})")
            }
        }
    }
}

impl std::error::Error for BroadcastError {}

/// Why a double spend of pending transactions was refused
///
/// The node replaces pending transactions the way Bitcoin Core 28 and later do
/// by default, with full RBF: the replaced transactions need not signal BIP 125,
/// but the replacement must pay for them under BIP 125's other rules
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ReplacementError {
    /// An input spends an output of a transaction the replacement would evict
    ///
    /// Not a BIP 125 rule: Core refuses it before the replacement rules run
    SpendsConflictingTx(Txid),
    /// Rule 2: an input spends a pending output that no evicted transaction spent from
    AddsUnconfirmedInput(OutPoint),
    /// Rule 3: the replacement pays less than every transaction it evicts together
    LessFeeThanReplaced { fee: Amount, replaced_fees: Amount },
    /// Rule 4: the extra fee does not pay to relay the replacement at the incremental rate
    NotEnoughToRelay { additional_fee: Amount, required: Amount },
    /// Rule 6: the fee rate is not above that of a transaction it directly conflicts with
    FeeRateNotHigher { conflict: Txid },
    /// Rule 5: the direct conflicts and their descendants add up to more than 100
    TooManyReplacements { count: usize },
    /// A transaction it would evict was added with [`FakeChain::add_to_mempool`] without
    /// known inputs, so the fees to beat are unknown
    UnknownFee(Txid),
}

/// Messages start with Bitcoin Core's reject reason for each rule
impl fmt::Display for ReplacementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpendsConflictingTx(txid) => {
                write!(f, "bad-txns-spends-conflicting-tx ({txid})")
            }

            Self::AddsUnconfirmedInput(outpoint) => {
                write!(f, "replacement-adds-unconfirmed ({outpoint})")
            }

            Self::LessFeeThanReplaced { fee, replaced_fees } => write!(
                f,
                "insufficient fee, less fees than conflicting txs; {} < {}",
                fee.to_sat(),
                replaced_fees.to_sat()
            ),

            Self::NotEnoughToRelay { additional_fee, required } => write!(
                f,
                "insufficient fee, not enough additional fees to relay; {} < {}",
                additional_fee.to_sat(),
                required.to_sat()
            ),

            Self::FeeRateNotHigher { conflict } => {
                write!(f, "insufficient fee, new feerate is not above that of {conflict}")
            }

            Self::TooManyReplacements { count } => write!(
                f,
                "too many potential replacements ({count} > {MAX_REPLACEMENT_CANDIDATES})"
            ),

            Self::UnknownFee(txid) => {
                write!(f, "fake node can't price {txid}, which was added to the mempool unchecked")
            }
        }
    }
}

/// A merkle inclusion proof with sibling hashes in internal byte order
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MerkleProof {
    /// Height of the block that holds the transaction
    pub height: u32,
    /// Index of the transaction inside the block, the coinbase being 0
    pub position: usize,
    /// Sibling hashes from the leaf up to the root
    pub branch: Vec<[u8; 32]>,
}

#[derive(Debug)]
struct Block {
    txids: Vec<Txid>,
    header: Header,
}

#[derive(Debug)]
struct ChainState {
    blocks: Vec<Block>,
    txs: HashMap<Txid, ChainTx>,
    /// Mempool and confirmed transactions in the order they were first added
    order: Vec<Txid>,
    /// Outputs that synthetic funding inputs spend, so fees can be reported
    funding_outputs: HashMap<OutPoint, TxOut>,
    next_funding_index: u64,
    /// Transactions first accepted through a broadcast, in the order they arrived
    broadcasts: Vec<Txid>,
}

/// Deterministic chain state shared by the fake Esplora and Electrum servers
///
/// Block 0 is the real genesis block of the network, so a wallet's local
/// chain agrees with it. Later blocks hold a synthetic coinbase and the
/// transactions the test confirms, with headers that carry valid merkle
/// roots (nothing in BDK or the node clients checks proof of work)
#[derive(Debug, Clone)]
pub struct FakeChain {
    network: Network,
    state: Arc<Mutex<ChainState>>,
}

impl FakeChain {
    /// Creates a chain that contains only the genesis block of `network`
    pub fn new(network: Network) -> Self {
        Self { network, state: Arc::new(Mutex::new(ChainState::new(network))) }
    }

    /// Drops every block after genesis, every transaction and the broadcast log
    ///
    /// Clones of this chain see the reset too, since they share its state
    pub fn reset(&self) {
        *self.state.lock() = ChainState::new(self.network);
    }

    /// The network whose genesis block the chain starts from
    pub const fn network(&self) -> Network {
        self.network
    }

    /// Height of the newest block, 0 when the chain holds only genesis
    pub fn tip_height(&self) -> u32 {
        self.state.lock().tip_height()
    }

    /// Hash of the newest block
    pub fn tip_hash(&self) -> bitcoin::BlockHash {
        let state = self.state.lock();

        state.blocks[state.tip_height() as usize].header.block_hash()
    }

    /// Appends `count` blocks that hold only their coinbase
    pub fn mine(&self, count: u32) {
        let mut state = self.state.lock();
        for _ in 0..count {
            state.push_block(&[]);
        }
    }

    /// Mines `count` blocks, the first of which confirms every pending
    /// transaction, and returns the new tip height
    pub fn mine_with_mempool(&self, count: u32) -> u32 {
        let mut state = self.state.lock();
        let pending = state
            .order
            .iter()
            .filter(|txid| state.txs[*txid].location == TxLocation::Mempool)
            .copied()
            .collect::<Vec<_>>();

        // insertion order keeps every parent ahead of its children in the block
        for block in 0..count {
            let txids = if block == 0 { pending.as_slice() } else { &[] };
            state.push_block(txids);
        }

        state.tip_height()
    }

    /// Adds a transaction to the mempool
    pub fn add_to_mempool(&self, tx: Transaction) -> Txid {
        self.state.lock().add_to_mempool(tx)
    }

    /// Accepts a transaction broadcast to the node
    ///
    /// Like bitcoind, a transaction already in the mempool is accepted again so
    /// a retried broadcast succeeds, while a confirmed one is rejected. Only the
    /// first acceptance is added to the broadcast log
    ///
    /// A new transaction must spend outputs the chain knows that no confirmed
    /// transaction spent, and every input's scripts and signatures must pass
    /// consensus checks. A double spend of pending transactions replaces them and
    /// their pending descendants when it meets the rules in [`ReplacementError`].
    /// Fees of other transactions, dust and standardness are not checked
    pub fn broadcast(&self, tx: Transaction) -> Result<Txid, BroadcastError> {
        let txid = tx.compute_txid();
        let mut state = self.state.lock();

        match state.txs.get(&txid).map(|entry| entry.location) {
            Some(TxLocation::Mempool) => return Ok(txid),
            Some(TxLocation::Confirmed { .. }) => return Err(BroadcastError::AlreadyConfirmed),
            None => {}
        }

        state.check_inputs_unspent(&tx)?;
        let fee = state.fee(&tx).ok_or(BroadcastError::OutputsExceedInputs)?;
        let replaced = state.check_replacement(&tx, fee).map_err(BroadcastError::Replacement)?;
        state.verify_scripts(&tx)?;

        state.remove(&replaced);
        state.add_to_mempool(tx);
        state.broadcasts.push(txid);

        Ok(txid)
    }

    /// Transactions accepted through a broadcast, oldest first
    ///
    /// An evicted transaction drops out of the log along with the chain
    pub fn broadcasts(&self) -> Vec<Transaction> {
        let state = self.state.lock();

        state
            .broadcasts
            .iter()
            .filter_map(|txid| state.txs.get(txid).map(|entry| entry.tx.clone()))
            .collect()
    }

    /// The pending or confirmed transaction that spends `outpoint`, if any
    pub fn spender_of(&self, outpoint: &OutPoint) -> Option<Txid> {
        let state = self.state.lock();

        state.order.iter().copied().find(|txid| {
            state.txs[txid].tx.input.iter().any(|input| input.previous_output == *outpoint)
        })
    }

    /// Pays `amount` to `script` from a synthetic funding output and leaves the
    /// transaction in the mempool
    pub fn pay(&self, script: &ScriptBuf, amount: Amount) -> Txid {
        let tx = {
            let mut state = self.state.lock();
            let index = state.next_funding_index;
            state.next_funding_index += 1;

            let funding_txid =
                Txid::from_byte_array(sha256::Hash::hash(&index.to_be_bytes()).to_byte_array());
            let funding_outpoint = OutPoint { txid: funding_txid, vout: 0 };
            let funding_output = TxOut {
                value: amount + FUNDING_FEE,
                script_pubkey: ScriptBuf::builder().push_opcode(OP_PUSHNUM_1).into_script(),
            };
            state.funding_outputs.insert(funding_outpoint, funding_output);

            Transaction {
                version: TxVersion::TWO,
                lock_time: LockTime::ZERO,
                input: vec![TxIn {
                    previous_output: funding_outpoint,
                    script_sig: ScriptBuf::new(),
                    sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                    witness: Witness::new(),
                }],
                output: vec![TxOut { value: amount, script_pubkey: script.clone() }],
            }
        };

        self.add_to_mempool(tx)
    }

    /// Mines one block that confirms `txid` and returns its height
    pub fn confirm(&self, txid: Txid) -> u32 {
        self.confirm_together(&[txid])
    }

    /// Mines one block that confirms every transaction in `txids`
    pub fn confirm_together(&self, txids: &[Txid]) -> u32 {
        let mut state = self.state.lock();
        for txid in txids {
            let entry = state.txs.get(txid).unwrap_or_else(|| panic!("unknown transaction {txid}"));
            assert_eq!(entry.location, TxLocation::Mempool, "{txid} is already confirmed");
        }

        state.push_block(txids)
    }

    /// Drops a transaction from the mempool, as a node does when it evicts it
    ///
    /// Its pending descendants go with it, since their inputs no longer exist
    pub fn evict(&self, txid: Txid) {
        let mut state = self.state.lock();
        let entry = state.txs.get(&txid).unwrap_or_else(|| panic!("unknown transaction {txid}"));
        assert_eq!(
            entry.location,
            TxLocation::Mempool,
            "{txid} is confirmed and cannot be evicted"
        );

        let evicted = state.with_pending_descendants(vec![txid]);
        state.remove(&evicted);
    }

    /// Where `txid` is, `None` when the chain has never seen it or evicted it
    pub fn location(&self, txid: Txid) -> Option<TxLocation> {
        self.state.lock().txs.get(&txid).map(|entry| entry.location)
    }

    /// Whether a pending transaction spends an output of another pending one,
    /// which bitcoind reports as a non-empty `depends` list
    pub fn has_unconfirmed_parent(&self, txid: Txid) -> bool {
        let state = self.state.lock();
        let Some(entry) = state.txs.get(&txid) else { return false };

        entry.tx.input.iter().any(|input| {
            state
                .txs
                .get(&input.previous_output.txid)
                .is_some_and(|parent| parent.location == TxLocation::Mempool)
        })
    }

    /// The transaction with `txid` and where it is in the chain
    pub fn tx(&self, txid: Txid) -> Option<ChainTx> {
        self.state.lock().txs.get(&txid).cloned()
    }

    /// The previous output of `outpoint` when the chain knows it
    pub fn prevout(&self, outpoint: &OutPoint) -> Option<TxOut> {
        self.state.lock().prevout(outpoint)
    }

    /// The previous outputs paying to `script` that the inputs of `tx` spend
    pub fn spent_from(&self, tx: &Transaction, script: &ScriptBuf) -> Vec<TxOut> {
        let state = self.state.lock();

        tx.input
            .iter()
            .filter_map(|input| state.prevout(&input.previous_output))
            .filter(|output| &output.script_pubkey == script)
            .collect()
    }

    /// The header of the block at `height`
    pub fn header(&self, height: u32) -> Option<Header> {
        self.state.lock().blocks.get(height as usize).map(|block| block.header)
    }

    /// The transactions of the block at `height`, coinbase first
    pub fn block_txids(&self, height: u32) -> Option<Vec<Txid>> {
        self.state.lock().blocks.get(height as usize).map(|block| block.txids.clone())
    }

    /// The tip block header and height, newest first for `count` blocks
    /// starting at `start_height`
    pub fn headers_descending(
        &self,
        start_height: Option<u32>,
        count: usize,
    ) -> Vec<(u32, Header)> {
        let state = self.state.lock();
        let start = start_height.unwrap_or_else(|| state.tip_height()).min(state.tip_height());

        (0..=start)
            .rev()
            .take(count)
            .map(|height| (height, state.blocks[height as usize].header))
            .collect()
    }

    /// The height of the block with `hash`
    pub fn height_of_block(&self, hash: bitcoin::BlockHash) -> Option<u32> {
        let state = self.state.lock();

        state
            .blocks
            .iter()
            .position(|block| block.header.block_hash() == hash)
            .map(|height| height as u32)
    }

    /// Every transaction that pays to or spends from `script`
    pub fn history(&self, script: &ScriptBuf) -> Vec<HistoryEntry> {
        let state = self.state.lock();

        state
            .order
            .iter()
            .filter_map(|txid| state.txs.get(txid).map(|entry| (txid, entry)))
            .filter(|(_, entry)| state.touches_script(&entry.tx, script))
            .map(|(txid, entry)| HistoryEntry { txid: *txid, location: entry.location })
            .collect()
    }

    /// The balance of `script` over its history
    ///
    /// Reads the history and its transactions under one lock, so a test that
    /// evicts a transaction mid-request never leaves an entry without its
    /// transaction
    pub fn balance(&self, script: &ScriptBuf) -> Balance {
        let state = self.state.lock();
        let mut balance = Balance::default();

        let entries = state.order.iter().filter_map(|txid| state.txs.get(txid));
        for entry in entries.filter(|entry| state.touches_script(&entry.tx, script)) {
            let received = entry
                .tx
                .output
                .iter()
                .filter(|output| &output.script_pubkey == script)
                .map(|output| output.value)
                .sum::<Amount>();
            let spent = entry
                .tx
                .input
                .iter()
                .filter_map(|input| state.prevout(&input.previous_output))
                .filter(|output| &output.script_pubkey == script)
                .map(|output| output.value)
                .sum::<Amount>();
            let change = received.to_signed().expect("amount fits").to_sat()
                - spent.to_signed().expect("amount fits").to_sat();

            match entry.location {
                TxLocation::Mempool => balance.unconfirmed += change,
                TxLocation::Confirmed { .. } => balance.confirmed += change,
            }
        }

        balance
    }

    /// Maps the SHA-256 digest of a script back to a script the chain has seen
    ///
    /// The digest is in hash output order; each protocol converts its own wire
    /// format first, so a client that sends the wrong byte order finds nothing
    pub fn script_for_digest(&self, digest: &[u8; 32]) -> Option<ScriptBuf> {
        let state = self.state.lock();
        let known_scripts = state.txs.values().flat_map(|entry| {
            let outputs = entry.tx.output.iter().map(|output| output.script_pubkey.clone());
            let inputs = entry
                .tx
                .input
                .iter()
                .filter_map(|input| state.prevout(&input.previous_output))
                .map(|output| output.script_pubkey);

            outputs.chain(inputs).collect::<Vec<_>>()
        });

        known_scripts
            .into_iter()
            .find(|script| sha256::Hash::hash(script.as_bytes()).to_byte_array() == *digest)
    }

    /// Proof that `txid` is in its block, `None` while it is pending
    pub fn merkle_proof(&self, txid: Txid) -> Option<MerkleProof> {
        let state = self.state.lock();
        let entry = state.txs.get(&txid)?;
        let TxLocation::Confirmed { height } = entry.location else { return None };
        let position = entry.position?;
        let leaves = state.blocks[height as usize].txids.iter().map(|txid| txid.to_byte_array());

        Some(MerkleProof { height, position, branch: merkle_branch(leaves.collect(), position) })
    }

    /// The fee a transaction pays, when every input's previous output is known and the
    /// outputs add up to no more than the inputs
    pub fn fee(&self, tx: &Transaction) -> Option<Amount> {
        self.state.lock().fee(tx)
    }
}

impl ChainState {
    fn new(network: Network) -> Self {
        let genesis = Block { txids: Vec::new(), header: genesis_block(network).header };

        Self {
            blocks: vec![genesis],
            txs: HashMap::new(),
            order: Vec::new(),
            funding_outputs: HashMap::new(),
            next_funding_index: 0,
            broadcasts: Vec::new(),
        }
    }

    fn add_to_mempool(&mut self, tx: Transaction) -> Txid {
        let txid = tx.compute_txid();
        assert!(!self.txs.contains_key(&txid), "transaction {txid} is already in the fake chain");

        self.order.push(txid);
        self.txs.insert(txid, ChainTx { tx, location: TxLocation::Mempool, position: None });

        txid
    }

    fn tip_height(&self) -> u32 {
        (self.blocks.len() - 1) as u32
    }

    fn prevout(&self, outpoint: &OutPoint) -> Option<TxOut> {
        self.funding_outputs.get(outpoint).cloned().or_else(|| {
            let entry = self.txs.get(&outpoint.txid)?;

            entry.tx.output.get(outpoint.vout as usize).cloned()
        })
    }

    fn fee(&self, tx: &Transaction) -> Option<Amount> {
        // checked sums, since `Sum for Amount` panics on overflow and any test can build a
        // transaction with absurd output values
        let total_in = tx.input.iter().try_fold(Amount::ZERO, |total, input| {
            total.checked_add(self.prevout(&input.previous_output)?.value)
        })?;
        let total_out = tx
            .output
            .iter()
            .try_fold(Amount::ZERO, |total, output| total.checked_add(output.value))?;

        total_in.checked_sub(total_out)
    }

    /// Core's policy size: the virtual size, raised for transactions heavy in signature
    /// operations to [`BYTES_PER_SIGOP`] per operation (`GetVirtualTransactionSize`)
    fn policy_vsize(&self, tx: &Transaction) -> u64 {
        let sigop_cost = tx.total_sigop_cost(|outpoint| self.prevout(outpoint)) as u64;
        let weight = tx.weight().to_wu().max(sigop_cost.saturating_mul(BYTES_PER_SIGOP));

        weight.div_ceil(4)
    }

    /// The fee of a pending transaction the replacement would evict
    fn evicted_fee(&self, txid: Txid) -> Result<Amount, ReplacementError> {
        self.fee(&self.txs[&txid].tx).ok_or(ReplacementError::UnknownFee(txid))
    }

    /// Pending transactions that spend an output `tx` also spends
    fn pending_conflicts(&self, tx: &Transaction) -> Vec<Txid> {
        self.order
            .iter()
            .copied()
            .filter(|txid| {
                let entry = &self.txs[txid];

                entry.location == TxLocation::Mempool
                    && entry.tx.input.iter().any(|spent| {
                        tx.input.iter().any(|input| input.previous_output == spent.previous_output)
                    })
            })
            .collect()
    }

    /// `roots` followed by every pending transaction that descends from one of them
    fn with_pending_descendants(&self, mut roots: Vec<Txid>) -> Vec<Txid> {
        let mut next = 0;
        while let Some(&parent) = roots.get(next) {
            let children = self
                .order
                .iter()
                .copied()
                .filter(|child| {
                    let entry = &self.txs[child];

                    entry.location == TxLocation::Mempool
                        && !roots.contains(child)
                        && entry.tx.input.iter().any(|input| input.previous_output.txid == parent)
                })
                .collect::<Vec<_>>();

            roots.extend(children);
            next += 1;
        }

        roots
    }

    fn remove(&mut self, txids: &[Txid]) {
        self.txs.retain(|txid, _| !txids.contains(txid));
        self.order.retain(|ordered| !txids.contains(ordered));
    }

    /// Refuses inputs the chain has never seen or that a confirmed transaction spent
    fn check_inputs_unspent(&self, tx: &Transaction) -> Result<(), BroadcastError> {
        for input in &tx.input {
            let outpoint = input.previous_output;
            if self.prevout(&outpoint).is_none() {
                return Err(BroadcastError::MissingInput(outpoint));
            }

            let spent_by_confirmed = self.txs.values().any(|entry| {
                matches!(entry.location, TxLocation::Confirmed { .. })
                    && entry.tx.input.iter().any(|spent| spent.previous_output == outpoint)
            });

            if spent_by_confirmed {
                return Err(BroadcastError::SpentInput(outpoint));
            }
        }

        Ok(())
    }

    /// The pending transactions `tx` would evict, after checking it pays to replace them
    ///
    /// Follows Bitcoin Core 28's full RBF policy and runs the checks in Core's order, see
    /// [`ReplacementError`]. `fee` is what `tx` pays
    fn check_replacement(
        &self,
        tx: &Transaction,
        fee: Amount,
    ) -> Result<Vec<Txid>, ReplacementError> {
        let conflicts = self.pending_conflicts(tx);
        if conflicts.is_empty() {
            return Ok(Vec::new());
        }

        let replaced = self.with_pending_descendants(conflicts.clone());

        // spending from an evicted transaction would leave the replacement without its input
        let spends_replaced =
            tx.input.iter().find(|input| replaced.contains(&input.previous_output.txid));

        if let Some(input) = spends_replaced {
            return Err(ReplacementError::SpendsConflictingTx(input.previous_output.txid));
        }

        // rule 6: Core compares rates per virtual byte, truncated to whole sat/kvB
        let vsize = self.policy_vsize(tx);
        let fee_rate = sat_per_kvb(fee, vsize);
        for conflict in &conflicts {
            let conflict_fee = self.evicted_fee(*conflict)?;
            let conflict_rate =
                sat_per_kvb(conflict_fee, self.policy_vsize(&self.txs[conflict].tx));

            if fee_rate <= conflict_rate {
                return Err(ReplacementError::FeeRateNotHigher { conflict: *conflict });
            }
        }

        // rule 5 counts each conflict with its descendants, the way Core's
        // `GetEntriesForConflicts` does, so a shared descendant counts once per conflict
        let count = conflicts
            .iter()
            .map(|conflict| self.with_pending_descendants(vec![*conflict]).len())
            .sum::<usize>();

        if count > MAX_REPLACEMENT_CANDIDATES {
            return Err(ReplacementError::TooManyReplacements { count });
        }

        // rule 2: a pending parent must already be the parent of a transaction it evicts,
        // descendants included
        let parents_of_replaced = replaced
            .iter()
            .flat_map(|txid| self.txs[txid].tx.input.iter())
            .map(|input| input.previous_output.txid)
            .collect::<HashSet<_>>();

        for input in &tx.input {
            let parent = input.previous_output.txid;
            let parent_is_pending =
                self.txs.get(&parent).is_some_and(|entry| entry.location == TxLocation::Mempool);

            if parent_is_pending && !parents_of_replaced.contains(&parent) {
                return Err(ReplacementError::AddsUnconfirmedInput(input.previous_output));
            }
        }

        // rule 3 counts the fees of descendants too, since they leave the mempool as well;
        // a total past `Amount::MAX` is more than any replacement can pay
        let mut replaced_fees = Amount::ZERO;
        for txid in &replaced {
            let evicted_fee = self.evicted_fee(*txid)?;
            replaced_fees = replaced_fees.checked_add(evicted_fee).unwrap_or(Amount::MAX);
        }

        let Some(additional_fee) = fee.checked_sub(replaced_fees) else {
            return Err(ReplacementError::LessFeeThanReplaced { fee, replaced_fees });
        };

        // rule 4
        let required = Amount::from_sat(INCREMENTAL_RELAY_SAT_PER_VB.saturating_mul(vsize));

        if additional_fee < required {
            return Err(ReplacementError::NotEnoughToRelay { additional_fee, required });
        }

        Ok(replaced)
    }

    /// Runs every input of `tx` through libbitcoinconsensus against the output it spends
    fn verify_scripts(&self, tx: &Transaction) -> Result<(), BroadcastError> {
        let spent = tx
            .input
            .iter()
            .map(|input| {
                let outpoint = input.previous_output;
                self.prevout(&outpoint).ok_or(BroadcastError::MissingInput(outpoint))
            })
            .collect::<Result<Vec<_>, _>>()?;

        // taproot signatures commit to every spent output, so each check gets all of them;
        // the pointers borrow from `spent`, which outlives every call below
        let spent_outputs = spent
            .iter()
            .map(|output| bitcoinconsensus::Utxo {
                script_pubkey: output.script_pubkey.as_bytes().as_ptr(),
                script_pubkey_len: u32::try_from(output.script_pubkey.len())
                    .expect("a script is shorter than 4 GB"),
                value: output.value.to_signed().expect("an amount fits in i64").to_sat(),
            })
            .collect::<Vec<_>>();
        let serialized = serialize(tx);

        for (index, output) in spent.iter().enumerate() {
            bitcoinconsensus::verify(
                output.script_pubkey.as_bytes(),
                output.value.to_sat(),
                &serialized,
                Some(&spent_outputs),
                index,
            )
            .map_err(|error| BroadcastError::InvalidScript { index, error })?;
        }

        Ok(())
    }

    fn touches_script(&self, tx: &Transaction, script: &ScriptBuf) -> bool {
        tx.output.iter().any(|output| &output.script_pubkey == script)
            || tx.input.iter().any(|input| {
                self.prevout(&input.previous_output)
                    .is_some_and(|output| &output.script_pubkey == script)
            })
    }

    /// Mines a block holding a coinbase followed by `txids`, returns its height
    fn push_block(&mut self, txids: &[Txid]) -> u32 {
        let height = self.blocks.len() as u32;
        let coinbase = coinbase_tx(height);
        let coinbase_txid = coinbase.compute_txid();
        let mut block_txids = vec![coinbase_txid];
        block_txids.extend_from_slice(txids);

        self.txs.insert(
            coinbase_txid,
            ChainTx { tx: coinbase, location: TxLocation::Confirmed { height }, position: Some(0) },
        );
        for (index, txid) in txids.iter().enumerate() {
            let entry = self.txs.get_mut(txid).expect("confirmed transactions are known");
            entry.location = TxLocation::Confirmed { height };
            entry.position = Some(index + 1);
        }

        let leaves = block_txids.iter().map(|txid| txid.to_byte_array()).collect::<Vec<_>>();
        let header = Header {
            version: BlockVersion::from_consensus(0x2000_0000),
            prev_blockhash: self.blocks[height as usize - 1].header.block_hash(),
            merkle_root: TxMerkleNode::from_byte_array(merkle_root(leaves)),
            time: FIRST_BLOCK_TIME + height * BLOCK_INTERVAL_SECS,
            bits: CompactTarget::from_consensus(EASY_BITS),
            nonce: height,
        };
        self.blocks.push(Block { txids: block_txids, header });

        height
    }
}

fn coinbase_tx(height: u32) -> Transaction {
    Transaction {
        version: TxVersion::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::builder().push_int(i64::from(height)).into_script(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(50 * 100_000_000),
            script_pubkey: ScriptBuf::builder().push_opcode(OP_PUSHNUM_1).into_script(),
        }],
    }
}

fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut joined = [0u8; 64];
    joined[..32].copy_from_slice(left);
    joined[32..].copy_from_slice(right);

    sha256d::Hash::hash(&joined).to_byte_array()
}

/// The merkle root of `leaves`, with an odd last node paired with itself
fn merkle_root(mut level: Vec<[u8; 32]>) -> [u8; 32] {
    while level.len() > 1 {
        level = next_level(&level);
    }

    level[0]
}

fn next_level(level: &[[u8; 32]]) -> Vec<[u8; 32]> {
    level.chunks(2).map(|pair| hash_pair(&pair[0], pair.get(1).unwrap_or(&pair[0]))).collect()
}

/// A fee rate the way Core's `CFeeRate` stores it: whole sat/kvB, truncated
///
/// Computed in u128 so no fee and size can overflow
fn sat_per_kvb(fee: Amount, vsize: u64) -> u128 {
    u128::from(fee.to_sat()) * 1_000 / u128::from(vsize.max(1))
}

/// The sibling hashes needed to walk from the leaf at `index` up to the root
fn merkle_branch(mut level: Vec<[u8; 32]>, mut index: usize) -> Vec<[u8; 32]> {
    let mut branch = Vec::new();
    while level.len() > 1 {
        let sibling = if index.is_multiple_of(2) { index + 1 } else { index - 1 };
        branch.push(*level.get(sibling).unwrap_or(&level[index]));

        level = next_level(&level);
        index /= 2;
    }

    branch
}
