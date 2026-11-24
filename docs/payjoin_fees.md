# Payjoin Fees

A Payjoin receiver adds one of its own coins to the payment, which makes the
transaction larger. Keeping the payment's fee rate then costs more. BIP 78 lets
the sender let the receiver take that extra fee from the sender's change, up to
a maximum the sender names (`maxadditionalfeecontribution`). Cove decides that
maximum in Rust before the user confirms, so the confirm screen can show the
most the payment can cost and no second approval is needed.

## The Setting

`PayjoinFeeHelp` is a global setting (`GlobalConfigKey::PayjoinFeeHelp`,
stored as `up_to_one_input` or `dont_help`) chosen in Settings > Payjoin:

- `UpToOneInput`, the default: offer up to the fee of one more input of the
  sending wallet's type at the payment's fee rate, the amount BIP 78 and most
  wallets offer
- `DontHelp`: offer nothing; the payment pays exactly the fee shown, and a
  receiver that will not pay for its own coin declines, so the original is sent

Frontends read it with `Database().globalConfig().payjoinFeeHelp()`, change it
with `AppAction::ChangePayjoinFeeHelp`, and follow
`AppStateReconcileMessage::PayjoinFeeHelpChanged`.

## The Offer

`PayjoinFeeContribution` exists only for a payment that offers something: fee
help is on, the payment has exactly one change output, and that change can give
more than nothing while staying spendable. The send flow builds it when it
finalizes a hot-wallet Payjoin payment and carries it in `PayjoinIntent`, so
the figures the confirm screen shows are the ones the payment sends.

The maximum is `min(fee_rate × input_weight, change − dust_limit)`:

| Wallet type    | Input weight     | At 10 sat/vB |
| -------------- | ---------------- | ------------ |
| Native SegWit  | 272 wu (68 vB)   | 680 sats     |
| Wrapped SegWit | 364 wu (91 vB)   | 910 sats     |
| Legacy         | 592 wu (148 vB)  | 1,480 sats   |

The fee rate is the one the user picked, and fractional results round up. The
dust cap exists because the receiver lowers the change by what it takes
without checking dust.

The sender is built with `build_with_additional_fee(max, Some(change_index),
fee_rate, false)`. With no offer it uses `build_with_additional_fee(0, None,
fee_rate, true)`: a payment with change then sends a zero maximum, which
reveals nothing extra with a single recipient, and a payment without change
sends none. rust-payjoin rejects a proposal that takes more than the maximum,
and a rejected or failed proposal broadcasts the original transaction.

## What the Confirm Screen Shows

When the payment is a Payjoin, the confirm screen has a per-payment Payjoin
switch, on by default, placed above the network fee and total rows it
changes. Turning it off sends the payment as a normal transaction. While it is
on and the payment has an offer, the network fee row adds "+ up to N with Payjoin" (`PayjoinFeeContribution::max`), and the total
reads "up to" `max_spending_amount`.

## The Fee Actually Paid

When a Payjoin payment completes, `PayjoinStatus::Completed` carries
`PayjoinFeePaid`: `total` is what the wallet spent beyond the payment itself,
and `fee_help` is the part the receiver took for its coin. The success alert
shows both.

BDK cannot compute a Payjoin transaction's fee, since it does not know the
value of the receiver's input, so the wallet stores the fee of each Payjoin it
broadcasts in `WalletData` under `payjoin_fees`, keyed by txid, and transaction
details read the fee from there.
A Payjoin whose session was cleaned up after a crash, before its broadcast was
recorded, has no stored fee and shows the fee BDK computes.
