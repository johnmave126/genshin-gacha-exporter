pub mod summary;

use std::io::{self, Write};

use crate::data_type::Pull;

/// Trait for generating analysis on gacha log
pub trait Report<'a> {
    /// Creating the report from a list of pulls
    fn new(log: &'a Vec<Pull>) -> Self;
    /// Display report in the console, by default we use `write` to
    /// display non-styled report
    fn print(&self) {
        self.write(&mut io::stdout()).unwrap();
    }
    /// Write report without style to something that implements `Write`
    fn write<T: Write>(&self, output: &mut T) -> io::Result<()>;
}
