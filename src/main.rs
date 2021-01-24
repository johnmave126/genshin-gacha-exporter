mod client;
mod data_type;
mod export;
mod report;
mod style;

use std::{env::current_dir, path::PathBuf};

use anyhow::{anyhow, Context};
use chrono::Local;
use console::style;
use dialoguer::{Confirm, Input, Select};
use reqwest::Url;

use crate::{
    client::Client,
    export::export_csv,
    report::{summary::Summary, Report},
    style::{init as init_style, THEME},
};

async fn run() -> anyhow::Result<()> {
    init_style();

    let url: Url = Input::with_theme(&*THEME)
        .with_prompt("请输入网址")
        .validate_with(|input: &String| -> anyhow::Result<()> {
            // input must be a url and something from in-game client
            let url = Url::parse(input).map_err(|err| anyhow!("输入不是网址: {}", err))?;
            if Client::verify_url(&url) {
                Ok(())
            } else {
                Err(anyhow!("输入网址不是有效的抽卡记录网址"))
            }
        })
        .interact()?
        .parse()
        .unwrap();

    let client = Client::new(url).await.context("初始化客户端失败")?;
    let pools = client.get_pools();

    loop {
        let selection: usize = Select::with_theme(&*THEME)
            .with_prompt("请选择需要查询的卡池")
            .items(pools)
            .item("退出")
            .default(0)
            .interact()?;

        // if the last one is selected, exit
        if selection == pools.len() {
            break;
        }
        let pool = &pools[selection];
        let log = client
            .request_gacha_log(pool)
            .await
            .context("获取抽卡记录失败")?;
        let summary = Summary::new(&log);
        summary.print();

        if Confirm::with_theme(&*THEME)
            .with_prompt("是否导出抽卡记录")
            .wait_for_newline(true)
            .default(true)
            .interact()?
        {
            // default being under cwd
            let mut save_path = current_dir().unwrap_or_default();
            save_path.push(format!(
                "{}-{}.csv",
                Local::now().format("%Y-%m-%d %H-%M-%S"),
                pool.name,
            ));
            let save_path = Input::with_theme(&*THEME)
                .with_prompt("保存位置")
                .validate_with(|path: &String| -> anyhow::Result<()> {
                    path.parse::<PathBuf>()?;
                    Ok(())
                })
                .with_initial_text(save_path.display().to_string())
                .interact()?;
            // make sure the extension is csv
            let save_path = PathBuf::from(save_path).with_extension("csv");
            export_csv(&log, &save_path).context("保存文件失败")?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // catch any error and display it
    if let Err(err) = run().await {
        eprintln!("{}{:?}", style("错误: ").red(), err);
        Input::<String>::new()
            .with_prompt("按回车键退出")
            .allow_empty(true)
            .interact()?;
        Err(err)
    } else {
        Ok(())
    }
}
