use std::{
    fmt,
    hash::{Hash, Hasher},
    ptr,
};

use chrono::{DateTime, Local};
use enum_map::Enum;

#[derive(Debug, Enum, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, Enum, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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
#[derive(Debug)]
pub struct Item {
    pub name: String,
    pub item_type: ItemType,
    pub rarity: Rarity,
}

impl<'a> Hash for &'a Item {
    fn hash<H>(&self, hasher: &mut H)
    where
        H: Hasher,
    {
        ptr::hash(*self as *const Item, hasher);
    }
}

impl<'a> PartialEq for &'a Item {
    fn eq(&self, other: &Self) -> bool {
        (*self as *const Item) == (*other as *const Item)
    }
}

impl<'a> Eq for &'a Item {}

/// result of a single gacha
#[derive(Debug)]
pub struct Pull<'a> {
    pub time: DateTime<Local>,
    pub item: &'a Item,
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
