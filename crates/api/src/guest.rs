//! One-click guest accounts: random names and word lists.
//!
//! Names follow `<adjective>_<noun>_<3 digits>`. The lists are fixed at compile
//! time so tests can assert the shape without mocking randomness.

use argon2::password_hash::rand_core::{OsRng, RngCore};

/// How many times to try another name when the case-insensitive index fires.
pub const MAX_NAME_ATTEMPTS: usize = 5;

/// A generated guest name and the Argon2 hash of a secret nobody will ever know.
pub struct GuestCredentials {
    pub username: String,
    pub display_name: String,
    pub password_hash: String,
}

/// Build a guest row: random name, random thrown-away password hash.
pub fn new_guest_credentials() -> Result<GuestCredentials, crate::auth::AuthError> {
    let username = generate_guest_name();
    let display_name = username.clone();
    let password_hash = hash_random_secret()?;
    Ok(GuestCredentials {
        username,
        display_name,
        password_hash,
    })
}

/// Pick adjective, noun and three digits. Exposed for tests.
pub fn generate_guest_name() -> String {
    let adj = ADJECTIVES[pick(ADJECTIVES.len())];
    let noun = NOUNS[pick(NOUNS.len())];
    let digits = pick(1000);
    format!("{adj}_{noun}_{digits:03}")
}

/// Whether a string matches the guest name pattern.
pub fn is_guest_name_format(name: &str) -> bool {
    let parts: Vec<&str> = name.split('_').collect();
    if parts.len() != 3 {
        return false;
    }
    let [adj, noun, digits] = [parts[0], parts[1], parts[2]];
    ADJECTIVES.contains(&adj)
        && NOUNS.contains(&noun)
        && digits.len() == 3
        && digits.chars().all(|c| c.is_ascii_digit())
}

fn pick(upper: usize) -> usize {
    (OsRng.next_u32() as usize) % upper
}

fn hash_random_secret() -> Result<String, crate::auth::AuthError> {
    let mut secret = [0u8; 32];
    OsRng.fill_bytes(&mut secret);
    let hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    crate::auth::hash_password(&hex)
}

const ADJECTIVES: &[&str] = &[
    "able", "active", "agile", "alert", "amber", "ample", "awake", "basic", "bold", "brave",
    "brief", "bright", "brisk", "calm", "candid", "careful", "cheerful", "chief", "civil", "clean",
    "clear", "clever", "close", "cool", "cosmic", "cozy", "crisp", "curious", "daily", "daring",
    "dear", "deep", "direct", "distant", "diverse", "driven", "eager", "early", "earnest", "easy",
    "elated", "elegant", "elite", "emerald", "even", "exact", "fair", "famous", "fancy", "fast",
    "fine", "firm", "first", "fleet", "fluent", "fond", "formal", "frank", "fresh", "friendly",
    "full", "funny", "gentle", "giant", "glad", "global", "golden", "good", "grand", "great",
    "green", "happy", "hardy", "hasty", "hearty", "helpful", "honest", "humble", "ideal", "inner",
    "jolly", "just", "keen", "kind", "known", "large", "last", "late", "lavish", "lawful",
    "lean", "level", "light", "likely", "lively", "local", "logical", "loyal", "lucky", "major",
    "mellow", "merry", "mighty", "mild", "modern", "modest", "moral", "mutual", "narrow", "native",
    "neat", "new", "nice", "noble", "normal", "noted", "novel", "odd", "open", "orange",
    "outer", "patient", "peaceful", "plain", "plucky", "polite", "popular", "precise", "pretty", "prime",
    "proud", "public", "pure", "quick", "quiet", "rapid", "rare", "ready", "real", "refined",
    "regular", "rich", "right", "robust", "rosy", "round", "royal", "rural", "safe", "sage",
    "sandy", "secure", "serene", "sharp", "shiny", "short", "silent", "simple", "sincere", "sleek",
    "slim", "smart", "smooth", "social", "soft", "solid", "sound", "spare", "special", "stable",
    "steady", "still", "stout", "strong", "subtle", "sunny", "super", "sure", "sweet", "swift",
    "tall", "tame", "tender", "thankful", "tidy", "tight", "tiny", "top", "tough", "true",
    "trusty", "unique", "upbeat", "useful", "valid", "vast", "verbal", "vivid", "warm", "whole",
    "wide", "wild", "wise", "witty", "worthy", "young", "zesty", "zippy", "aglow", "amiable",
    "ardent", "artful", "astute", "avid", "balmy", "beaming", "blithe", "bonny", "breezy", "buoyant",
    "canny", "charming", "chipper", "comely", "comfy", "cordial", "dainty", "dapper", "dashing", "deft",
    "devoted", "diligent", "dreamy", "droll", "earthy", "ebullient", "effusive", "ethereal", "expert", "fab",
    "fabled", "factual", "fearless", "feisty", "festive", "fiery", "flashy", "floral",
];

const NOUNS: &[&str] = &[
    "acorn", "anchor", "apple", "arrow", "atlas", "badge", "balloon", "banana", "beacon", "bear",
    "beetle", "berry", "bird", "bison", "blaze", "bloom", "boat", "book", "boot", "bowl",
    "branch", "bridge", "brook", "brush", "bubble", "bucket", "butter", "button", "cabin", "cactus",
    "cake", "camel", "candle", "canoe", "canyon", "cape", "card", "carp", "castle", "cat",
    "cedar", "chair", "chalk", "charm", "cheese", "cherry", "chest", "chip", "clam", "claw",
    "cliff", "cloud", "clover", "coach", "coast", "cobra", "coin", "comet", "coral", "cork",
    "corn", "crab", "crane", "crate", "creek", "crest", "crow", "crown", "crystal", "cube",
    "daisy", "dawn", "deck", "deer", "delta", "dew", "diamond", "dish", "dock", "dolphin",
    "donkey", "door", "dove", "dragon", "drum", "duck", "dune", "eagle", "earth", "echo",
    "elm", "ember", "engine", "falcon", "feather", "fern", "field", "finch", "flag", "flame",
    "flask", "flea", "flint", "flute", "fog", "forest", "fork", "fox", "frame", "frog",
    "frost", "fruit", "garden", "gate", "gem", "giraffe", "glade", "glass", "globe", "glow",
    "goat", "goose", "grape", "grass", "grove", "gull", "hammer", "hare", "harbor", "hawk",
    "hay", "heart", "hedge", "heron", "hill", "hive", "honey", "horse", "house", "ibis",
    "ice", "igloo", "ink", "iris", "island", "ivy", "jade", "jaguar", "jar", "jay",
    "jewel", "jungle", "kayak", "kettle", "key", "kite", "kitten", "kiwi", "knight", "koala",
    "lake", "lamp", "lark", "leaf", "lemon", "lily", "lime", "lion", "lizard", "lobster",
    "lock", "lotus", "lynx", "mango", "maple", "marble", "meadow", "melon", "mica", "mint",
    "mirror", "mist", "mole", "moon", "moose", "moss", "moth", "mouse", "mushroom", "nest",
    "nettle", "night", "noodle", "oak", "ocean", "olive", "onion", "orange", "orchid", "otter",
    "owl", "oyster", "panda", "panel", "panther", "paper", "parrot", "path", "peach", "pearl",
    "pebble", "penguin", "pepper", "petal", "piano", "pickle", "pigeon", "pillow", "pine", "planet",
    "plate", "plum", "pond", "pony", "pool", "poppy", "prism", "puffin", "puppy", "quail",
    "quartz", "queen", "quill", "rabbit", "raccoon", "raft", "rain", "raven", "reef", "rhino",
    "ribbon", "river", "robin", "rocket", "rose", "ruby", "rug", "sail", "salmon", "sand",
    "sapphire", "sardine", "scale", "scarf", "seal", "seed", "shark", "shell", "shield", "ship",
    "shoe", "shore", "shrub", "sign", "silver", "skate", "skunk", "sky", "sled", "snail",
    "snake", "snow", "soap", "sock", "sofa", "song", "spark", "sparrow", "spear", "spider",
    "spoon", "spruce", "squid", "squirrel", "star", "stone", "storm", "stream", "sun", "swan",
    "swing", "table", "tiger", "toast", "token", "tomato", "tower", "trail", "train", "tree",
    "trout", "tulip", "tuna", "turtle", "valley", "vase", "veil", "vine", "violet", "violin",
    "vista", "volcano", "wagon", "walnut", "wave", "whale", "wheat", "wheel", "willow", "wind",
    "wolf", "wood", "wren", "yacht", "yarn", "zebra", "zephyr", "zone", "zoom",
];
