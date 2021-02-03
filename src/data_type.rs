use std::{fmt, hash::Hash};

use chrono::{DateTime, Local};
use enum_map::Enum;

#[derive(Debug, Enum, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemType {
    Weapon,
    Character,
}

impl fmt::Display for ItemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Weapon => write!(f, "武器"),
            Self::Character => write!(f, "角色"),
        }
    }
}

#[derive(Debug, Enum, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rarity {
    Three,
    Four,
    Five,
}

impl fmt::Display for Rarity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Three => write!(f, "3"),
            Self::Four => write!(f, "4"),
            Self::Five => write!(f, "5"),
        }
    }
}

/// information of an item
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Item {
    pub name: String,
    pub item_type: ItemType,
    pub rarity: Rarity,
}

/// result of a single gacha
#[derive(Debug)]
pub struct Pull {
    pub time: DateTime<Local>,
    pub item: Item,
}

/// information of a gacha pool
#[derive(Debug)]
pub struct Pool {
    pub id: usize,
    pub key: String,
    pub name: String,
}

impl ToString for Pool {
    fn to_string(&self) -> String {
        self.name.clone()
    }
}
