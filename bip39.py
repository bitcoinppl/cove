
# BIP39 mnemonics
# - should work desktop and mpy

from hashlib import sha256

# from https://raw.githubusercontent.com/bitcoin/bips/master/bip-0039/english.txt
# each word = 11 bits => 2048 words
wordlist_en = (
    'abandon', 'ability', 'able', 'about', 'above', 'absent', 'absorb', 'abstract',
    'absurd', 'abuse', 'access', 'accident', 'account', 'accuse', 'achieve', 'acid',
    'acoustic', 'acquire', 'across', 'act', 'action', 'actor', 'actress', 'actual',
    'adapt', 'add', 'addict', 'address', 'adjust', 'admit', 'adult', 'advance',
    'advice', 'aerobic', 'affair', 'afford', 'afraid', 'again', 'age', 'agent',
    'agree', 'ahead', 'aim', 'air', 'airport', 'aisle', 'alarm', 'album', 'alcohol',
    'alert', 'alien', 'all', 'alley', 'allow', 'almost', 'alone', 'alpha', 'already',
    'also', 'alter', 'always', 'amateur', 'amazing', 'among', 'amount', 'amused',
    'analyst', 'anchor', 'ancient', 'anger', 'angle', 'angry', 'animal', 'ankle',
    'announce', 'annual', 'another', 'answer', 'antenna', 'antique', 'anxiety', 'any',
    'apart', 'apology', 'appear', 'apple', 'approve', 'april', 'arch', 'arctic',
    'area', 'arena', 'argue', 'arm', 'armed', 'armor', 'army', 'around', 'arrange',
    'arrest', 'arrive', 'arrow', 'art', 'artefact', 'artist', 'artwork', 'ask',
    'aspect', 'assault', 'asset', 'assist', 'assume', 'asthma', 'athlete', 'atom',
    'attack', 'attend', 'attitude', 'attract', 'auction', 'audit', 'august', 'aunt',
    'author', 'auto', 'autumn', 'average', 'avocado', 'avoid', 'awake', 'aware',
    'away', 'awesome', 'awful', 'awkward', 'axis', 'baby', 'bachelor', 'bacon',
    'badge', 'bag', 'balance', 'balcony', 'ball', 'bamboo', 'banana', 'banner', 'bar',
    'barely', 'bargain', 'barrel', 'base', 'basic', 'basket', 'battle', 'beach',
    'bean', 'beauty', 'because', 'become', 'beef', 'before', 'begin', 'behave',
    'behind', 'believe', 'below', 'belt', 'bench', 'benefit', 'best', 'betray',
    'better', 'between', 'beyond', 'bicycle', 'bid', 'bike', 'bind', 'biology',
    'bird', 'birth', 'bitter', 'black', 'blade', 'blame', 'blanket', 'blast', 'bleak',
    'bless', 'blind', 'blood', 'blossom', 'blouse', 'blue', 'blur', 'blush', 'board',
    'boat', 'body', 'boil', 'bomb', 'bone', 'bonus', 'book', 'boost', 'border',
    'boring', 'borrow', 'boss', 'bottom', 'bounce', 'box', 'boy', 'bracket', 'brain',
    'brand', 'brass', 'brave', 'bread', 'breeze', 'brick', 'bridge', 'brief',
    'bright', 'bring', 'brisk', 'broccoli', 'broken', 'bronze', 'broom', 'brother',
    'brown', 'brush', 'bubble', 'buddy', 'budget', 'buffalo', 'build', 'bulb', 'bulk',
    'bullet', 'bundle', 'bunker', 'burden', 'burger', 'burst', 'bus', 'business',
    'busy', 'butter', 'buyer', 'buzz', 'cabbage', 'cabin', 'cable', 'cactus', 'cage',
    'cake', 'call', 'calm', 'camera', 'camp', 'can', 'canal', 'cancel', 'candy',
    'cannon', 'canoe', 'canvas', 'canyon', 'capable', 'capital', 'captain', 'car',
    'carbon', 'card', 'cargo', 'carpet', 'carry', 'cart', 'case', 'cash', 'casino',
    'castle', 'casual', 'cat', 'catalog', 'catch', 'category', 'cattle', 'caught',
    'cause', 'caution', 'cave', 'ceiling', 'celery', 'cement', 'census', 'century',
    'cereal', 'certain', 'chair', 'chalk', 'champion', 'change', 'chaos', 'chapter',
    'charge', 'chase', 'chat', 'cheap', 'check', 'cheese', 'chef', 'cherry', 'chest',
    'chicken', 'chief', 'child', 'chimney', 'choice', 'choose', 'chronic', 'chuckle',
    'chunk', 'churn', 'cigar', 'cinnamon', 'circle', 'citizen', 'city', 'civil',
    'claim', 'clap', 'clarify', 'claw', 'clay', 'clean', 'clerk', 'clever', 'click',
    'client', 'cliff', 'climb', 'clinic', 'clip', 'clock', 'clog', 'close', 'cloth',
    'cloud', 'clown', 'club', 'clump', 'cluster', 'clutch', 'coach', 'coast',
    'coconut', 'code', 'coffee', 'coil', 'coin', 'collect', 'color', 'column',
    'combine', 'come', 'comfort', 'comic', 'common', 'company', 'concert', 'conduct',
    'confirm', 'congress', 'connect', 'consider', 'control', 'convince', 'cook',
    'cool', 'copper', 'copy', 'coral', 'core', 'corn', 'correct', 'cost', 'cotton',
    'couch', 'country', 'couple', 'course', 'cousin', 'cover', 'coyote', 'crack',
    'cradle', 'craft', 'cram', 'crane', 'crash', 'crater', 'crawl', 'crazy', 'cream',
    'credit', 'creek', 'crew', 'cricket', 'crime', 'crisp', 'critic', 'crop', 'cross',
    'crouch', 'crowd', 'crucial', 'cruel', 'cruise', 'crumble', 'crunch', 'crush',
    'cry', 'crystal', 'cube', 'culture', 'cup', 'cupboard', 'curious', 'current',
    'curtain', 'curve', 'cushion', 'custom', 'cute', 'cycle', 'dad', 'damage', 'damp',
    'dance', 'danger', 'daring', 'dash', 'daughter', 'dawn', 'day', 'deal', 'debate',
    'debris', 'decade', 'december', 'decide', 'decline', 'decorate', 'decrease',
    'deer', 'defense', 'define', 'defy', 'degree', 'delay', 'deliver', 'demand',
    'demise', 'denial', 'dentist', 'deny', 'depart', 'depend', 'deposit', 'depth',
    'deputy', 'derive', 'describe', 'desert', 'design', 'desk', 'despair', 'destroy',
    'detail', 'detect', 'develop', 'device', 'devote', 'diagram', 'dial', 'diamond',
    'diary', 'dice', 'diesel', 'diet', 'differ', 'digital', 'dignity', 'dilemma',
    'dinner', 'dinosaur', 'direct', 'dirt', 'disagree', 'discover', 'disease', 'dish',
    'dismiss', 'disorder', 'display', 'distance', 'divert', 'divide', 'divorce',
    'dizzy', 'doctor', 'document', 'dog', 'doll', 'dolphin', 'domain', 'donate',
    'donkey', 'donor', 'door', 'dose', 'double', 'dove', 'draft', 'dragon', 'drama',
    'drastic', 'draw', 'dream', 'dress', 'drift', 'drill', 'drink', 'drip', 'drive',
    'drop', 'drum', 'dry', 'duck', 'dumb', 'dune', 'during', 'dust', 'dutch', 'duty',
    'dwarf', 'dynamic', 'eager', 'eagle', 'early', 'earn', 'earth', 'easily', 'east',
    'easy', 'echo', 'ecology', 'economy', 'edge', 'edit', 'educate', 'effort', 'egg',
    'eight', 'either', 'elbow', 'elder', 'electric', 'elegant', 'element', 'elephant',
    'elevator', 'elite', 'else', 'embark', 'embody', 'embrace', 'emerge', 'emotion',
    'employ', 'empower', 'empty', 'enable', 'enact', 'end', 'endless', 'endorse',
    'enemy', 'energy', 'enforce', 'engage', 'engine', 'enhance', 'enjoy', 'enlist',
    'enough', 'enrich', 'enroll', 'ensure', 'enter', 'entire', 'entry', 'envelope',
    'episode', 'equal', 'equip', 'era', 'erase', 'erode', 'erosion', 'error', 'erupt',
    'escape', 'essay', 'essence', 'estate', 'eternal', 'ethics', 'evidence', 'evil',
    'evoke', 'evolve', 'exact', 'example', 'excess', 'exchange', 'excite', 'exclude',
    'excuse', 'execute', 'exercise', 'exhaust', 'exhibit', 'exile', 'exist', 'exit',
    'exotic', 'expand', 'expect', 'expire', 'explain', 'expose', 'express', 'extend',
    'extra', 'eye', 'eyebrow', 'fabric', 'face', 'faculty', 'fade', 'faint', 'faith',
    'fall', 'false', 'fame', 'family', 'famous', 'fan', 'fancy', 'fantasy', 'farm',
    'fashion', 'fat', 'fatal', 'father', 'fatigue', 'fault', 'favorite', 'feature',
    'february', 'federal', 'fee', 'feed', 'feel', 'female', 'fence', 'festival',
    'fetch', 'fever', 'few', 'fiber', 'fiction', 'field', 'figure', 'file', 'film',
    'filter', 'final', 'find', 'fine', 'finger', 'finish', 'fire', 'firm', 'first',
    'fiscal', 'fish', 'fit', 'fitness', 'fix', 'flag', 'flame', 'flash', 'flat',
    'flavor', 'flee', 'flight', 'flip', 'float', 'flock', 'floor', 'flower', 'fluid',
    'flush', 'fly', 'foam', 'focus', 'fog', 'foil', 'fold', 'follow', 'food', 'foot',
    'force', 'forest', 'forget', 'fork', 'fortune', 'forum', 'forward', 'fossil',
    'foster', 'found', 'fox', 'fragile', 'frame', 'frequent', 'fresh', 'friend',
    'fringe', 'frog', 'front', 'frost', 'frown', 'frozen', 'fruit', 'fuel', 'fun',
    'funny', 'furnace', 'fury', 'future', 'gadget', 'gain', 'galaxy', 'gallery',
    'game', 'gap', 'garage', 'garbage', 'garden', 'garlic', 'garment', 'gas', 'gasp',
    'gate', 'gather', 'gauge', 'gaze', 'general', 'genius', 'genre', 'gentle',
    'genuine', 'gesture', 'ghost', 'giant', 'gift', 'giggle', 'ginger', 'giraffe',
    'girl', 'give', 'glad', 'glance', 'glare', 'glass', 'glide', 'glimpse', 'globe',
    'gloom', 'glory', 'glove', 'glow', 'glue', 'goat', 'goddess', 'gold', 'good',
    'goose', 'gorilla', 'gospel', 'gossip', 'govern', 'gown', 'grab', 'grace',
    'grain', 'grant', 'grape', 'grass', 'gravity', 'great', 'green', 'grid', 'grief',
    'grit', 'grocery', 'group', 'grow', 'grunt', 'guard', 'guess', 'guide', 'guilt',
    'guitar', 'gun', 'gym', 'habit', 'hair', 'half', 'hammer', 'hamster', 'hand',
    'happy', 'harbor', 'hard', 'harsh', 'harvest', 'hat', 'have', 'hawk', 'hazard',
    'head', 'health', 'heart', 'heavy', 'hedgehog', 'height', 'hello', 'helmet',
    'help', 'hen', 'hero', 'hidden', 'high', 'hill', 'hint', 'hip', 'hire', 'history',
    'hobby', 'hockey', 'hold', 'hole', 'holiday', 'hollow', 'home', 'honey', 'hood',
    'hope', 'horn', 'horror', 'horse', 'hospital', 'host', 'hotel', 'hour', 'hover',
    'hub', 'huge', 'human', 'humble', 'humor', 'hundred', 'hungry', 'hunt', 'hurdle',
    'hurry', 'hurt', 'husband', 'hybrid', 'ice', 'icon', 'idea', 'identify', 'idle',
    'ignore', 'ill', 'illegal', 'illness', 'image', 'imitate', 'immense', 'immune',
    'impact', 'impose', 'improve', 'impulse', 'inch', 'include', 'income', 'increase',
    'index', 'indicate', 'indoor', 'industry', 'infant', 'inflict', 'inform',
    'inhale', 'inherit', 'initial', 'inject', 'injury', 'inmate', 'inner', 'innocent',
    'input', 'inquiry', 'insane', 'insect', 'inside', 'inspire', 'install', 'intact',
    'interest', 'into', 'invest', 'invite', 'involve', 'iron', 'island', 'isolate',
    'issue', 'item', 'ivory', 'jacket', 'jaguar', 'jar', 'jazz', 'jealous', 'jeans',
    'jelly', 'jewel', 'job', 'join', 'joke', 'journey', 'joy', 'judge', 'juice',
    'jump', 'jungle', 'junior', 'junk', 'just', 'kangaroo', 'keen', 'keep', 'ketchup',
    'key', 'kick', 'kid', 'kidney', 'kind', 'kingdom', 'kiss', 'kit', 'kitchen',
    'kite', 'kitten', 'kiwi', 'knee', 'knife', 'knock', 'know', 'lab', 'label',
    'labor', 'ladder', 'lady', 'lake', 'lamp', 'language', 'laptop', 'large', 'later',
    'latin', 'laugh', 'laundry', 'lava', 'law', 'lawn', 'lawsuit', 'layer', 'lazy',
    'leader', 'leaf', 'learn', 'leave', 'lecture', 'left', 'leg', 'legal', 'legend',
    'leisure', 'lemon', 'lend', 'length', 'lens', 'leopard', 'lesson', 'letter',
    'level', 'liar', 'liberty', 'library', 'license', 'life', 'lift', 'light', 'like',
    'limb', 'limit', 'link', 'lion', 'liquid', 'list', 'little', 'live', 'lizard',
    'load', 'loan', 'lobster', 'local', 'lock', 'logic', 'lonely', 'long', 'loop',
    'lottery', 'loud', 'lounge', 'love', 'loyal', 'lucky', 'luggage', 'lumber',
    'lunar', 'lunch', 'luxury', 'lyrics', 'machine', 'mad', 'magic', 'magnet', 'maid',
    'mail', 'main', 'major', 'make', 'mammal', 'man', 'manage', 'mandate', 'mango',
    'mansion', 'manual', 'maple', 'marble', 'march', 'margin', 'marine', 'market',
    'marriage', 'mask', 'mass', 'master', 'match', 'material', 'math', 'matrix',
    'matter', 'maximum', 'maze', 'meadow', 'mean', 'measure', 'meat', 'mechanic',
    'medal', 'media', 'melody', 'melt', 'member', 'memory', 'mention', 'menu',
    'mercy', 'merge', 'merit', 'merry', 'mesh', 'message', 'metal', 'method',
    'middle', 'midnight', 'milk', 'million', 'mimic', 'mind', 'minimum', 'minor',
    'minute', 'miracle', 'mirror', 'misery', 'miss', 'mistake', 'mix', 'mixed',
    'mixture', 'mobile', 'model', 'modify', 'mom', 'moment', 'monitor', 'monkey',
    'monster', 'month', 'moon', 'moral', 'more', 'morning', 'mosquito', 'mother',
    'motion', 'motor', 'mountain', 'mouse', 'move', 'movie', 'much', 'muffin', 'mule',
    'multiply', 'muscle', 'museum', 'mushroom', 'music', 'must', 'mutual', 'myself',
    'mystery', 'myth', 'naive', 'name', 'napkin', 'narrow', 'nasty', 'nation',
    'nature', 'near', 'neck', 'need', 'negative', 'neglect', 'neither', 'nephew',
    'nerve', 'nest', 'net', 'network', 'neutral', 'never', 'news', 'next', 'nice',
    'night', 'noble', 'noise', 'nominee', 'noodle', 'normal', 'north', 'nose',
    'notable', 'note', 'nothing', 'notice', 'novel', 'now', 'nuclear', 'number',
    'nurse', 'nut', 'oak', 'obey', 'object', 'oblige', 'obscure', 'observe', 'obtain',
    'obvious', 'occur', 'ocean', 'october', 'odor', 'off', 'offer', 'office', 'often',
    'oil', 'okay', 'old', 'olive', 'olympic', 'omit', 'once', 'one', 'onion',
    'online', 'only', 'open', 'opera', 'opinion', 'oppose', 'option', 'orange',
    'orbit', 'orchard', 'order', 'ordinary', 'organ', 'orient', 'original', 'orphan',
    'ostrich', 'other', 'outdoor', 'outer', 'output', 'outside', 'oval', 'oven',
    'over', 'own', 'owner', 'oxygen', 'oyster', 'ozone', 'pact', 'paddle', 'page',
    'pair', 'palace', 'palm', 'panda', 'panel', 'panic', 'panther', 'paper', 'parade',
    'parent', 'park', 'parrot', 'party', 'pass', 'patch', 'path', 'patient', 'patrol',
    'pattern', 'pause', 'pave', 'payment', 'peace', 'peanut', 'pear', 'peasant',
    'pelican', 'pen', 'penalty', 'pencil', 'people', 'pepper', 'perfect', 'permit',
    'person', 'pet', 'phone', 'photo', 'phrase', 'physical', 'piano', 'picnic',
    'picture', 'piece', 'pig', 'pigeon', 'pill', 'pilot', 'pink', 'pioneer', 'pipe',
    'pistol', 'pitch', 'pizza', 'place', 'planet', 'plastic', 'plate', 'play',
    'please', 'pledge', 'pluck', 'plug', 'plunge', 'poem', 'poet', 'point', 'polar',
    'pole', 'police', 'pond', 'pony', 'pool', 'popular', 'portion', 'position',
    'possible', 'post', 'potato', 'pottery', 'poverty', 'powder', 'power', 'practice',
    'praise', 'predict', 'prefer', 'prepare', 'present', 'pretty', 'prevent', 'price',
    'pride', 'primary', 'print', 'priority', 'prison', 'private', 'prize', 'problem',
    'process', 'produce', 'profit', 'program', 'project', 'promote', 'proof',
    'property', 'prosper', 'protect', 'proud', 'provide', 'public', 'pudding', 'pull',
    'pulp', 'pulse', 'pumpkin', 'punch', 'pupil', 'puppy', 'purchase', 'purity',
    'purpose', 'purse', 'push', 'put', 'puzzle', 'pyramid', 'quality', 'quantum',
    'quarter', 'question', 'quick', 'quit', 'quiz', 'quote', 'rabbit', 'raccoon',
    'race', 'rack', 'radar', 'radio', 'rail', 'rain', 'raise', 'rally', 'ramp',
    'ranch', 'random', 'range', 'rapid', 'rare', 'rate', 'rather', 'raven', 'raw',
    'razor', 'ready', 'real', 'reason', 'rebel', 'rebuild', 'recall', 'receive',
    'recipe', 'record', 'recycle', 'reduce', 'reflect', 'reform', 'refuse', 'region',
    'regret', 'regular', 'reject', 'relax', 'release', 'relief', 'rely', 'remain',
    'remember', 'remind', 'remove', 'render', 'renew', 'rent', 'reopen', 'repair',
    'repeat', 'replace', 'report', 'require', 'rescue', 'resemble', 'resist',
    'resource', 'response', 'result', 'retire', 'retreat', 'return', 'reunion',
    'reveal', 'review', 'reward', 'rhythm', 'rib', 'ribbon', 'rice', 'rich', 'ride',
    'ridge', 'rifle', 'right', 'rigid', 'ring', 'riot', 'ripple', 'risk', 'ritual',
    'rival', 'river', 'road', 'roast', 'robot', 'robust', 'rocket', 'romance', 'roof',
    'rookie', 'room', 'rose', 'rotate', 'rough', 'round', 'route', 'royal', 'rubber',
    'rude', 'rug', 'rule', 'run', 'runway', 'rural', 'sad', 'saddle', 'sadness',
    'safe', 'sail', 'salad', 'salmon', 'salon', 'salt', 'salute', 'same', 'sample',
    'sand', 'satisfy', 'satoshi', 'sauce', 'sausage', 'save', 'say', 'scale', 'scan',
    'scare', 'scatter', 'scene', 'scheme', 'school', 'science', 'scissors',
    'scorpion', 'scout', 'scrap', 'screen', 'script', 'scrub', 'sea', 'search',
    'season', 'seat', 'second', 'secret', 'section', 'security', 'seed', 'seek',
    'segment', 'select', 'sell', 'seminar', 'senior', 'sense', 'sentence', 'series',
    'service', 'session', 'settle', 'setup', 'seven', 'shadow', 'shaft', 'shallow',
    'share', 'shed', 'shell', 'sheriff', 'shield', 'shift', 'shine', 'ship', 'shiver',
    'shock', 'shoe', 'shoot', 'shop', 'short', 'shoulder', 'shove', 'shrimp', 'shrug',
    'shuffle', 'shy', 'sibling', 'sick', 'side', 'siege', 'sight', 'sign', 'silent',
    'silk', 'silly', 'silver', 'similar', 'simple', 'since', 'sing', 'siren',
    'sister', 'situate', 'six', 'size', 'skate', 'sketch', 'ski', 'skill', 'skin',
    'skirt', 'skull', 'slab', 'slam', 'sleep', 'slender', 'slice', 'slide', 'slight',
    'slim', 'slogan', 'slot', 'slow', 'slush', 'small', 'smart', 'smile', 'smoke',
    'smooth', 'snack', 'snake', 'snap', 'sniff', 'snow', 'soap', 'soccer', 'social',
    'sock', 'soda', 'soft', 'solar', 'soldier', 'solid', 'solution', 'solve',
    'someone', 'song', 'soon', 'sorry', 'sort', 'soul', 'sound', 'soup', 'source',
    'south', 'space', 'spare', 'spatial', 'spawn', 'speak', 'special', 'speed',
    'spell', 'spend', 'sphere', 'spice', 'spider', 'spike', 'spin', 'spirit', 'split',
    'spoil', 'sponsor', 'spoon', 'sport', 'spot', 'spray', 'spread', 'spring', 'spy',
    'square', 'squeeze', 'squirrel', 'stable', 'stadium', 'staff', 'stage', 'stairs',
    'stamp', 'stand', 'start', 'state', 'stay', 'steak', 'steel', 'stem', 'step',
    'stereo', 'stick', 'still', 'sting', 'stock', 'stomach', 'stone', 'stool',
    'story', 'stove', 'strategy', 'street', 'strike', 'strong', 'struggle', 'student',
    'stuff', 'stumble', 'style', 'subject', 'submit', 'subway', 'success', 'such',
    'sudden', 'suffer', 'sugar', 'suggest', 'suit', 'summer', 'sun', 'sunny',
    'sunset', 'super', 'supply', 'supreme', 'sure', 'surface', 'surge', 'surprise',
    'surround', 'survey', 'suspect', 'sustain', 'swallow', 'swamp', 'swap', 'swarm',
    'swear', 'sweet', 'swift', 'swim', 'swing', 'switch', 'sword', 'symbol',
    'symptom', 'syrup', 'system', 'table', 'tackle', 'tag', 'tail', 'talent', 'talk',
    'tank', 'tape', 'target', 'task', 'taste', 'tattoo', 'taxi', 'teach', 'team',
    'tell', 'ten', 'tenant', 'tennis', 'tent', 'term', 'test', 'text', 'thank',
    'that', 'theme', 'then', 'theory', 'there', 'they', 'thing', 'this', 'thought',
    'three', 'thrive', 'throw', 'thumb', 'thunder', 'ticket', 'tide', 'tiger', 'tilt',
    'timber', 'time', 'tiny', 'tip', 'tired', 'tissue', 'title', 'toast', 'tobacco',
    'today', 'toddler', 'toe', 'together', 'toilet', 'token', 'tomato', 'tomorrow',
    'tone', 'tongue', 'tonight', 'tool', 'tooth', 'top', 'topic', 'topple', 'torch',
    'tornado', 'tortoise', 'toss', 'total', 'tourist', 'toward', 'tower', 'town',
    'toy', 'track', 'trade', 'traffic', 'tragic', 'train', 'transfer', 'trap',
    'trash', 'travel', 'tray', 'treat', 'tree', 'trend', 'trial', 'tribe', 'trick',
    'trigger', 'trim', 'trip', 'trophy', 'trouble', 'truck', 'true', 'truly',
    'trumpet', 'trust', 'truth', 'try', 'tube', 'tuition', 'tumble', 'tuna', 'tunnel',
    'turkey', 'turn', 'turtle', 'twelve', 'twenty', 'twice', 'twin', 'twist', 'two',
    'type', 'typical', 'ugly', 'umbrella', 'unable', 'unaware', 'uncle', 'uncover',
    'under', 'undo', 'unfair', 'unfold', 'unhappy', 'uniform', 'unique', 'unit',
    'universe', 'unknown', 'unlock', 'until', 'unusual', 'unveil', 'update',
    'upgrade', 'uphold', 'upon', 'upper', 'upset', 'urban', 'urge', 'usage', 'use',
    'used', 'useful', 'useless', 'usual', 'utility', 'vacant', 'vacuum', 'vague',
    'valid', 'valley', 'valve', 'van', 'vanish', 'vapor', 'various', 'vast', 'vault',
    'vehicle', 'velvet', 'vendor', 'venture', 'venue', 'verb', 'verify', 'version',
    'very', 'vessel', 'veteran', 'viable', 'vibrant', 'vicious', 'victory', 'video',
    'view', 'village', 'vintage', 'violin', 'virtual', 'virus', 'visa', 'visit',
    'visual', 'vital', 'vivid', 'vocal', 'voice', 'void', 'volcano', 'volume', 'vote',
    'voyage', 'wage', 'wagon', 'wait', 'walk', 'wall', 'walnut', 'want', 'warfare',
    'warm', 'warrior', 'wash', 'wasp', 'waste', 'water', 'wave', 'way', 'wealth',
    'weapon', 'wear', 'weasel', 'weather', 'web', 'wedding', 'weekend', 'weird',
    'welcome', 'west', 'wet', 'whale', 'what', 'wheat', 'wheel', 'when', 'where',
    'whip', 'whisper', 'wide', 'width', 'wife', 'wild', 'will', 'win', 'window',
    'wine', 'wing', 'wink', 'winner', 'winter', 'wire', 'wisdom', 'wise', 'wish',
    'witness', 'wolf', 'woman', 'wonder', 'wood', 'wool', 'word', 'work', 'world',
    'worry', 'worth', 'wrap', 'wreck', 'wrestle', 'wrist', 'write', 'wrong', 'yard',
    'year', 'yellow', 'you', 'young', 'youth', 'zebra', 'zero', 'zone', 'zoo' )

'''
if 0:
    lookup = dict()
    for idx,w in enumerate(wordlist_en):
        if w[0] not in lookup:
            lookup[w[0]] = idx
        if w[0:2] not in lookup:            # 2k cost
            lookup[w[0:2]] = idx
        if 0 and w[0:3] not in lookup:      # 9k cost in .mpy
            lookup[w[0:3]] = idx
    print("_lookup = %r" % lookup)
    print("# %d entries" % len(lookup))
'''

_lookup = {'a':0, 'ab':0, 'ac':10, 'ad':24, 'ae':33, 'af':34, 'ag':37, 'ah':41, 'ai':42,
'al':46, 'am':61, 'an':66, 'ap':82, 'ar':88, 'as':106, 'at':113, 'au':119, 'av':126,
'aw':129, 'ax':135, 'b':136, 'ba':136, 'be':155, 'bi':175, 'bl':183, 'bo':197, 'br':214,
'bu':234, 'c':253, 'ca':253, 'ce':295, 'ch':302, 'ci':327, 'cl':333, 'co':357, 'cr':398,
'cu':427, 'cy':438, 'd':439, 'da':439, 'de':449, 'di':487, 'do':514, 'dr':527, 'du':542,
'dw':549, 'dy':550, 'e':551, 'ea':551, 'ec':559, 'ed':562, 'ef':565, 'eg':566, 'ei':567,
'el':569, 'em':578, 'en':586, 'ep':607, 'eq':608, 'er':610, 'es':616, 'et':620, 'ev':622,
'ex':626, 'ey':649, 'f':651, 'fa':651, 'fe':673, 'fi':685, 'fl':705, 'fo':720, 'fr':739,
'fu':751, 'g':757, 'ga':757, 'ge':774, 'gh':780, 'gi':781, 'gl':788, 'go':800, 'gr':810,
'gu':826, 'gy':832, 'h':833, 'ha':833, 'he':848, 'hi':859, 'ho':866, 'hu':884, 'hy':896,
'i':897, 'ic':897, 'id':899, 'ig':902, 'il':903, 'im':906, 'in':914, 'ir':946, 'is':947,
'it':950, 'iv':951, 'j':952, 'ja':952, 'je':956, 'jo':960, 'ju':965, 'k':972, 'ka':972,
'ke':973, 'ki':977, 'kn':988, 'l':992, 'la':992, 'le':1012, 'li':1030, 'lo':1047,
'lu':1061, 'ly':1067, 'm':1068, 'ma':1068, 'me':1101, 'mi':1122, 'mo':1139, 'mu':1160,
'my':1170, 'n':1173, 'na':1173, 'ne':1180, 'ni':1195, 'no':1197, 'nu':1210, 'o':1214,
'oa':1214, 'ob':1215, 'oc':1222, 'od':1225, 'of':1226, 'oi':1230, 'ok':1231, 'ol':1232,
'om':1235, 'on':1236, 'op':1241, 'or':1246, 'os':1255, 'ot':1256, 'ou':1257, 'ov':1261,
'ow':1264, 'ox':1266, 'oy':1267, 'oz':1268, 'p':1269, 'pa':1269, 'pe':1294, 'ph':1308,
'pi':1312, 'pl':1326, 'po':1336, 'pr':1355, 'pu':1384, 'py':1400, 'q':1401, 'qu':1401,
'r':1409, 'ra':1409, 're':1430, 'rh':1478, 'ri':1479, 'ro':1495, 'ru':1510, 's':1517,
'sa':1517, 'sc':1536, 'se':1551, 'sh':1574, 'si':1597, 'sk':1616, 'sl':1623, 'sm':1635,
'sn':1640, 'so':1645, 'sp':1666, 'sq':1691, 'st':1694, 'su':1727, 'sw':1752, 'sy':1763,
't':1767, 'ta':1767, 'te':1780, 'th':1790, 'ti':1805, 'to':1816, 'tr':1844, 'tu':1872,
'tw':1880, 'ty':1886, 'u':1888, 'ug':1888, 'um':1889, 'un':1890, 'up':1908, 'ur':1914,
'us':1916, 'ut':1922, 'v':1923, 'va':1923, 've':1935, 'vi':1946, 'vo':1962, 'w':1969,
'wa':1969, 'we':1985, 'wh':1997, 'wi':2005, 'wo':2022, 'wr':2032, 'y':2038, 'ya':2038,
'ye':2039, 'yo':2041, 'z':2044, 'ze':2044, 'zo':2046} # 225 entries


def get_word_index(word):
    "input string word; return int index in wordlist, or ValueError"
    # - also accepts just first four distinctive characters of the word

    try:
        # all words are 3..8 long
        assert 3 <= len(word) <= 8

        # first two letters must be right
        start_index = _lookup[word[:2]]
    except (KeyError, AssertionError):
        raise ValueError(word)

    for i, w in enumerate(wordlist_en[start_index:], start=start_index):
        if word == w or word == w[:4]:
            return i
        if w[0] != word[0]:
            break

    raise ValueError(word)


def _split_lookup(phrase):
    "decode & lookup only"

    if isinstance(phrase, str):
        phrase = phrase.split()

    num = len(phrase)

    rv = 0
    for w in phrase:
        idx = get_word_index(w)
        rv = (rv << 11) | idx

    return num, rv


def a2b_words_guess(phrase):
    "generate a list of possible final words"
    num, rv = _split_lookup(phrase)

    if num not in { 11, 14, 17, 20, 23 }:
        return

    # assume just one more word missing
    chk_w = (num+1) // 3
    width = (((num+1) * 11) - chk_w) // 8
    prv = rv << (11-chk_w)

    print(chk_w, width, prv)

    for i in range(1<<(11-chk_w)):
        rv = prv | i
        bits = rv.to_bytes(width, 'big')
        chk = sha256(bits).digest()[0] >> (8-chk_w)
        yield wordlist_en[(i<<chk_w) + chk]

if __name__ == '__main__':
    words = " ".join([
        "wrap",
        "jar",
        "physical",
        "abuse",
        "minimum",
        "sand",
        "hair",
        "pet",
        "address",
        "alley",
        "fashion",
        "thank",
        "duck",
        "sound",
        "budget",
        "spell",
        "flush",
        "knock",
        "source",
        "novel",
        "mixed",
        "detect",
        "tackle",
    ])

    print(words)
    print(_split_lookup(words))

    possible_words = list(a2b_words_guess(words))
    print(possible_words)
