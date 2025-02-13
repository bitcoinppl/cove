use serde::{Deserialize, Serialize};

/// A record in in the database with a timestamp
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Record<T> {
    pub item: T,
    pub timestamps: Timestamps,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timestamps {
    pub created_at: u64,
    pub updated_at: u64,
}

impl<T> Record<T> {
    pub fn new(item: T) -> Self {
        let now = jiff::Timestamp::now().as_second() as u64;
        Self {
            item,
            timestamps: Timestamps {
                created_at: now,
                updated_at: now,
            },
        }
    }

    pub fn with_timestamps(item: T, timestamps: Timestamps) -> Self {
        Self { item, timestamps }
    }

    pub fn created_at(&self) -> u64 {
        self.timestamps.created_at
    }

    pub fn updated_at(&self) -> u64 {
        self.timestamps.updated_at
    }
}

impl<T> Record<T> {
    pub fn into<U>(self) -> Record<U>
    where
        T: Into<U>,
    {
        Record {
            item: self.item.into(),
            timestamps: self.timestamps,
        }
    }
}

impl Timestamps {
    pub fn new(created_at: u64, updated_at: u64) -> Self {
        Self {
            created_at,
            updated_at,
        }
    }

    pub fn now() -> Self {
        let now = jiff::Timestamp::now().as_second() as u64;
        Self::new(now, now)
    }
}
