use std::{
    cmp,
    collections::HashMap,
    fmt,
    io::{self, Write},
};

use console::{style, StyledObject};
use enum_map::EnumMap;

use crate::{
    data_type::{Item, ItemType, Pull, Rarity},
    report::Report,
};

/// Contains a summary of basic stats regarding a gacha log
#[derive(Debug)]
pub struct Summary<'a> {
    /// total number of pulls
    pub len: usize,
    /// stats of correspondent rarity
    pub stats_per_rarity: EnumMap<Rarity, StatsForRarity<'a>>,
    /// stats of correspondent item type
    pub stats_per_type: EnumMap<ItemType, StatsForType>,
}

/// Helper enum so that a string can be write both to console with style and to file without style
enum StylizedString {
    Styled(StyledObject<String>),
    UnStyled(String),
}

impl fmt::Display for StylizedString {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Styled(obj) => write!(f, "{}", obj),
            Self::UnStyled(s) => write!(f, "{}", s),
        }
    }
}

impl StylizedString {
    /// Apply style specified by `f`, if the variant is UnStyled, nothing will happen
    fn with_style<F>(self, f: F) -> Self
    where
        F: FnOnce(StyledObject<String>) -> StyledObject<String>,
    {
        match self {
            Self::Styled(obj) => Self::Styled(f(obj)),
            s @ _ => s,
        }
    }
}

impl<'a> Summary<'a> {
    /// pretty print the summary
    fn write_to<T: Write>(&self, output: &mut T, with_style: bool) -> io::Result<()> {
        let stylizer: Box<dyn Fn(String) -> StylizedString> = if with_style {
            Box::new(|s| StylizedString::Styled(style(s)))
        } else {
            Box::new(|s| StylizedString::UnStyled(s))
        };

        writeln!(
            output,
            "你一共进行了{}抽，其中五星{}抽，四星{}抽，三星{}抽",
            stylizer(self.len.to_string()).with_style(StyledObject::blue),
            stylizer(self.stats_per_rarity[Rarity::Five].num.to_string())
                .with_style(StyledObject::yellow),
            stylizer(self.stats_per_rarity[Rarity::Four].num.to_string())
                .with_style(StyledObject::magenta),
            stylizer(self.stats_per_rarity[Rarity::Three].num.to_string())
                .with_style(StyledObject::blue),
        )?;
        writeln!(
            output,
            "综合出率五星{}%，四星{}%",
            stylizer(format!(
                "{:.2}",
                self.stats_per_rarity[Rarity::Five].num as f64 / self.len as f64 * 100.0
            ))
            .with_style(StyledObject::yellow),
            stylizer(format!(
                "{:.2}",
                self.stats_per_rarity[Rarity::Four].num as f64 / self.len as f64 * 100.0
            ))
            .with_style(StyledObject::magenta),
        )?;
        writeln!(
            output,
            "共抽出{}个武器，其中五星{}抽，四星{}抽",
            stylizer(self.stats_per_type[ItemType::Weapon].num.to_string())
                .with_style(StyledObject::blue),
            stylizer(
                self.stats_per_type[ItemType::Weapon].num_per_rarity[Rarity::Five].to_string()
            )
            .with_style(StyledObject::yellow),
            stylizer(
                self.stats_per_type[ItemType::Weapon].num_per_rarity[Rarity::Four].to_string()
            )
            .with_style(StyledObject::magenta),
        )?;
        writeln!(
            output,
            "共抽出{}个角色，其中五星{}抽，四星{}抽",
            stylizer(self.stats_per_type[ItemType::Character].num.to_string())
                .with_style(StyledObject::blue),
            stylizer(
                self.stats_per_type[ItemType::Character].num_per_rarity[Rarity::Five].to_string()
            )
            .with_style(StyledObject::yellow),
            stylizer(
                self.stats_per_type[ItemType::Character].num_per_rarity[Rarity::Four].to_string()
            )
            .with_style(StyledObject::magenta),
        )?;
        writeln!(
            output,
            "最多连续抽出{}个五星，连续抽出{}个四星",
            stylizer(
                self.stats_per_rarity[Rarity::Five]
                    .longest_streak
                    .to_string()
            )
            .with_style(StyledObject::yellow),
            stylizer(
                self.stats_per_rarity[Rarity::Four]
                    .longest_streak
                    .to_string()
            )
            .with_style(StyledObject::magenta),
        )?;
        writeln!(
            output,
            "最多{}抽未抽出五星，目前{}抽未抽出五星，{}抽未抽出四星",
            stylizer(
                self.stats_per_rarity[Rarity::Five]
                    .longest_drought
                    .to_string()
            )
            .with_style(StyledObject::red),
            stylizer(
                self.stats_per_rarity[Rarity::Five]
                    .current_drought
                    .to_string()
            )
            .with_style(StyledObject::red),
            stylizer(
                self.stats_per_rarity[Rarity::Four]
                    .current_drought
                    .to_string()
            )
            .with_style(StyledObject::red),
        )?;
        if let Some((item, count)) = self.stats_per_rarity[Rarity::Five]
            .sorted_occurrence
            .first()
        {
            writeln!(
                output,
                "抽出的五星中，{}出现次数最多，抽出{}次",
                stylizer(item.name.clone()).with_style(StyledObject::yellow),
                stylizer(count.to_string()).with_style(StyledObject::blue),
            )?;
        }
        if let Some((item, count)) = self.stats_per_rarity[Rarity::Four]
            .sorted_occurrence
            .first()
        {
            writeln!(
                output,
                "抽出的四星中，{}出现次数最多，抽出{}次",
                stylizer(item.name.clone()).with_style(StyledObject::magenta),
                stylizer(count.to_string()).with_style(StyledObject::blue),
            )?;
        }
        output.flush()?;
        Ok(())
    }
}

impl<'a> Report<'a> for Summary<'a> {
    fn new(log: &'a Vec<Pull>) -> Self {
        log.iter()
            .fold(IntermediateSummary::default(), |mut summary, pull| {
                summary.update(pull);
                summary
            })
            .into()
    }
    fn print(&self) {
        self.write_to(&mut io::stdout(), true).unwrap();
    }
    fn write<T: Write>(&self, output: &mut T) -> io::Result<()> {
        self.write_to(output, false)
    }
}

/// Intermediate summary while folding
#[derive(Debug, Default)]
struct IntermediateSummary<'a> {
    /// total number of pulls
    len: usize,
    /// stats of correspondent rarity
    stats_per_rarity: EnumMap<Rarity, IntermediateStatsForRarity<'a>>,
    /// stats of correspondent item type
    stats_per_type: EnumMap<ItemType, StatsForType>,
}

impl<'a> IntermediateSummary<'a> {
    fn update(&mut self, pull: &'a Pull) {
        self.len += 1;
        for (rarity, stats) in self.stats_per_rarity.iter_mut() {
            stats.update(rarity, pull);
        }
        self.stats_per_type[pull.item.item_type].update(pull);
    }
}

impl<'a> Into<Summary<'a>> for IntermediateSummary<'a> {
    fn into(self) -> Summary<'a> {
        let mut stats_per_rarity = EnumMap::new();
        stats_per_rarity.extend(
            self.stats_per_rarity
                .into_iter()
                .map(|(rarity, stats)| (rarity, stats.into())),
        );
        Summary {
            len: self.len,
            stats_per_rarity,
            stats_per_type: self.stats_per_type,
        }
    }
}

/// Statistics classified by rarity
#[derive(Default, Debug)]
pub struct StatsForRarity<'a> {
    /// total pulls in this rarity
    pub num: usize,
    pub current_streak: usize,
    pub longest_streak: usize,
    pub current_drought: usize,
    pub longest_drought: usize,
    pub sorted_occurrence: Vec<(&'a Item, usize)>,
}

/// Intermediate statistics classified by rarity
#[derive(Default, Debug)]
struct IntermediateStatsForRarity<'a> {
    /// total pulls in this rarity
    num: usize,
    current_streak: usize,
    longest_streak: usize,
    current_drought: usize,
    longest_drought: usize,
    occurrences: HashMap<&'a Item, usize>,
}

impl<'a> IntermediateStatsForRarity<'a> {
    fn update(&mut self, rarity: Rarity, pull: &'a Pull) {
        if rarity == pull.item.rarity {
            self.num += 1;
            self.current_streak += 1;
            self.longest_streak = cmp::max(self.current_streak, self.longest_streak);
            self.current_drought = 0;
            *self.occurrences.entry(pull.item).or_insert(0) += 1;
        } else {
            self.current_streak = 0;
            self.current_drought += 1;
            self.longest_drought = cmp::max(self.current_drought, self.longest_drought);
        }
    }
}

impl<'a> Into<StatsForRarity<'a>> for IntermediateStatsForRarity<'a> {
    fn into(mut self) -> StatsForRarity<'a> {
        let mut sorted_occurrence: Vec<(&'a Item, usize)> = self.occurrences.drain().collect();
        sorted_occurrence.sort_by_key(|(_, cnt)| cmp::Reverse(*cnt));
        StatsForRarity {
            num: self.num,
            current_streak: self.current_streak,
            longest_streak: self.longest_streak,
            current_drought: self.current_drought,
            longest_drought: self.longest_drought,
            sorted_occurrence,
        }
    }
}

#[derive(Default, Debug)]
pub struct StatsForType {
    pub num: usize,
    pub num_per_rarity: EnumMap<Rarity, usize>,
}

impl StatsForType {
    fn update(&mut self, pull: &Pull) {
        self.num += 1;
        self.num_per_rarity[pull.item.rarity] += 1;
    }
}
