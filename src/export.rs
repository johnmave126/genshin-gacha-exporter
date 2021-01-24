use indicatif::ProgressBar;
/// Functions that export a list of pulls to a file
use std::{fs::File, io, io::Write, path::Path};

use crate::{data_type::Pull, style::SPINNER_STYLE};

/// export a list of pulls into a csv file
pub fn export_csv(results: &[Pull], path: &Path) -> io::Result<()> {
    let pb = ProgressBar::new_spinner()
        .with_style(SPINNER_STYLE.clone().template("{spinner:.green} {msg}"));
    pb.set_message("正在导出");
    let mut output = File::create(path)?;
    // UTF-8 BOM
    output.write_all(&[0xEF, 0xBB, 0xBF])?;
    writeln!(output, "抽卡时间,抽卡结果,类型,稀有度")?;
    pb.tick();
    for pull in results.iter() {
        writeln!(
            output,
            "{},{},{},{}",
            pull.time.format("%Y-%m-%d %T"),
            pull.item.name,
            pull.item.item_type,
            pull.item.rarity
        )?;
        pb.tick();
    }
    pb.finish_with_message("导出完毕");
    Ok(())
}
