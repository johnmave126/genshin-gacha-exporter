use std::{
    cmp,
    collections::{HashMap, HashSet},
    env::current_dir,
    fs::File,
    future,
    io::Write,
    path::{Path, PathBuf},
    string::ToString,
};

use anyhow::{anyhow, Context};
use chrono::{DateTime, Local, TimeZone};
use console::style;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use futures::stream::{self, StreamExt, TryStreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use tokio::task::spawn_blocking;

#[derive(Debug, Serialize, Deserialize)]
struct GachaConfig {
    id: String,
    key: String,
    name: String,
}

impl ToString for GachaConfig {
    fn to_string(&self) -> String {
        self.name.clone()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ConfigListData {
    gacha_type_list: Vec<GachaConfig>,
    region: String,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
struct GachaResult {
    #[serde_as(as = "DisplayFromStr")]
    uid: usize,
    gacha_type: String,
    #[serde_as(as = "DisplayFromStr")]
    item_id: usize,
    count: String,
    time: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct GachaResultPage {
    page: String,
    size: String,
    total: String,
    list: Vec<GachaResult>,
    region: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiResponse<T> {
    retcode: i32,
    message: String,
    data: Option<T>,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
struct Item {
    #[serde_as(as = "DisplayFromStr")]
    item_id: usize,
    name: String,
    item_type: String,
    #[serde_as(as = "DisplayFromStr")]
    rank_type: u8,
}

const REQUIRED_FIELDS: &[&'static str] = &[
    "authkey_ver",
    "sign_type",
    "auth_appid",
    "gacha_id",
    "lang",
    "game_biz",
    "authkey",
    "region",
];

const ADDITIONAL_FIELDS: &[&'static str] = &["device_type", "ext", "game_version"];

const CONFIG_LIST_URL: &'static str =
    "https://hk4e-api-os.mihoyo.com/event/gacha_info/api/getConfigList";
const GACHA_LOG_URL: &'static str =
    "https://hk4e-api-os.mihoyo.com/event/gacha_info/api/getGachaLog";

fn item_list_url(region: &str, lang: &str) -> Url {
    Url::parse(&format!(
        "https://webstatic-sea.mihoyo.com/hk4e/gacha_info/{}/items/{}.json",
        region, lang
    ))
    .unwrap()
}

async fn get_config_list(
    client: &Client,
    raw_queries: &HashMap<String, String>,
    pb: ProgressBar,
) -> anyhow::Result<Vec<GachaConfig>> {
    pb.set_message("加载卡池列表");
    let url = Url::parse_with_params(CONFIG_LIST_URL, raw_queries).unwrap();
    let resp = client
        .get(url)
        .send()
        .await?
        .json::<ApiResponse<ConfigListData>>()
        .await?;
    if resp.retcode != 0 {
        return Err(anyhow!(resp.message));
    }
    pb.finish_with_message("已加载卡池列表");

    Ok(resp.data.unwrap().gacha_type_list)
}

async fn get_item_list(
    client: &Client,
    raw_queries: &HashMap<String, String>,
    pb: ProgressBar,
) -> anyhow::Result<Vec<Item>> {
    pb.set_message("加载图鉴");
    let region = raw_queries.get("region").unwrap();
    let lang = raw_queries.get("lang").unwrap();

    let url = item_list_url(region, lang);
    let result = client.get(url).send().await?.json::<Vec<Item>>().await?;
    pb.finish_with_message("已加载图鉴");
    Ok(result)
}

async fn get_gacha_result(
    client: &Client,
    raw_queries: &HashMap<String, String>,
    pool: &str,
    page: usize,
) -> anyhow::Result<Vec<GachaResult>> {
    let mut url = Url::parse_with_params(GACHA_LOG_URL, raw_queries).unwrap();
    url.query_pairs_mut()
        .append_pair("init_type", pool)
        .append_pair("gacha_type", pool)
        .append_pair("page", &page.to_string())
        .append_pair("size", "20");
    let resp = client
        .get(url)
        .send()
        .await?
        .json::<ApiResponse<GachaResultPage>>()
        .await?;
    if resp.retcode != 0 {
        return Err(anyhow!(resp.message));
    }

    Ok(resp.data.unwrap().list)
}

async fn get_gacha_result_all(
    client: &Client,
    raw_queries: &HashMap<String, String>,
    pool: &str,
    pb: ProgressBar,
) -> anyhow::Result<Vec<GachaResult>> {
    pb.set_message("正在加载，已");
    let result: Vec<GachaResult> = stream::iter(1..)
        .then(|page| get_gacha_result(client, raw_queries, pool, page))
        .try_take_while(|results| future::ready(Ok(results.len() > 0)))
        .and_then(|results| {
            pb.inc(results.len() as u64);
            future::ready(Ok(stream::iter(
                results
                    .into_iter()
                    .map(|result| Ok::<_, anyhow::Error>(result)),
            )))
        })
        .try_flatten()
        .try_collect()
        .await?;
    pb.set_length(result.len() as u64);
    pb.finish_with_message("已");
    Ok(result)
}

fn associate_items(
    item_list: &HashMap<usize, Item>,
    gacha_results: Vec<GachaResult>,
) -> Vec<(&Item, DateTime<Local>)> {
    gacha_results
        .into_iter()
        .filter_map(|result| {
            item_list.get(&result.item_id).map(|item| {
                (
                    item,
                    Local
                        .datetime_from_str(&result.time, "%Y-%m-%d %T")
                        .expect("invalid datetime format"),
                )
            })
        })
        .collect()
}

#[derive(Debug)]
struct Statistics<'a> {
    total: usize,
    star5: usize,
    star4: usize,
    star3: usize,
    longest_star5_streak: usize,
    longest_star5_drought: usize,
    current_star5_drought: usize,
    longest_star4_streak: usize,
    weapon: usize,
    weapon_star5: usize,
    weapon_star4: usize,
    character: usize,
    character_star5: usize,
    character_star4: usize,
    most_common_star5: Option<(&'a Item, usize)>,
    most_common_star4: Option<(&'a Item, usize)>,
}

impl<'a> Statistics<'a> {
    fn from(results: &'a Vec<(&'a Item, DateTime<Local>)>, weapon_ident: &str) -> Self {
        #[derive(Default)]
        struct IntermediateStats<'a> {
            total: usize,
            star5: usize,
            star4: usize,
            longest_star5_streak: usize,
            current_star5_streak: usize,
            longest_star5_drought: usize,
            current_star5_drought: usize,
            longest_star4_streak: usize,
            current_star4_streak: usize,
            weapon: usize,
            weapon_star5: usize,
            weapon_star4: usize,
            character: usize,
            character_star5: usize,
            character_star4: usize,
            individual_star5_count: HashMap<usize, (usize, &'a Item)>,
            individual_star4_count: HashMap<usize, (usize, &'a Item)>,
        }

        impl<'a> IntermediateStats<'a> {
            fn update(
                mut self,
                (item, _): &(&'a Item, DateTime<Local>),
                weapon_ident: &str,
            ) -> Self {
                let (weapon_delta, character_delta) = if item.item_type == weapon_ident {
                    (1, 0)
                } else {
                    (0, 1)
                };
                match item.rank_type {
                    5 => {
                        let current_star5_streak = self.current_star5_streak + 1;
                        self.individual_star5_count
                            .entry(item.item_id)
                            .or_insert((0, item))
                            .0 += 1;
                        Self {
                            total: self.total + 1,
                            star5: self.star5 + 1,
                            longest_star5_streak: cmp::max(
                                self.longest_star5_streak,
                                current_star5_streak,
                            ),
                            current_star5_streak,
                            current_star5_drought: 0,
                            current_star4_streak: 0,
                            weapon: self.weapon + weapon_delta,
                            weapon_star5: self.weapon_star5 + weapon_delta,
                            character: self.character + character_delta,
                            character_star5: self.character_star5 + character_delta,
                            ..self
                        }
                    }
                    4 => {
                        let current_star4_streak = self.current_star4_streak + 1;
                        let current_star5_drought = self.current_star5_drought + 1;
                        self.individual_star4_count
                            .entry(item.item_id)
                            .or_insert((0, item))
                            .0 += 1;
                        Self {
                            total: self.total + 1,
                            star4: self.star4 + 1,
                            current_star5_streak: 0,
                            longest_star5_drought: cmp::max(
                                current_star5_drought,
                                self.longest_star5_drought,
                            ),
                            current_star5_drought,
                            longest_star4_streak: cmp::max(
                                self.longest_star4_streak,
                                current_star4_streak,
                            ),
                            current_star4_streak,
                            weapon: self.weapon + weapon_delta,
                            weapon_star4: self.weapon_star4 + weapon_delta,
                            character: self.character + character_delta,
                            character_star4: self.character_star4 + character_delta,
                            ..self
                        }
                    }
                    3 => {
                        let current_star5_drought = self.current_star5_drought + 1;
                        Self {
                            total: self.total + 1,
                            current_star5_streak: 0,
                            longest_star5_drought: cmp::max(
                                current_star5_drought,
                                self.longest_star5_drought,
                            ),
                            current_star5_drought,
                            current_star4_streak: 0,
                            weapon: self.weapon + weapon_delta,
                            character: self.character + character_delta,
                            ..self
                        }
                    }
                    _ => unreachable!(),
                }
            }

            fn to_final_stat(self) -> Statistics<'a> {
                Statistics {
                    total: self.total,
                    star5: self.star5,
                    star4: self.star4,
                    star3: self.total - self.star5 - self.star4,
                    longest_star5_streak: self.longest_star5_streak,
                    longest_star5_drought: self.longest_star5_drought,
                    current_star5_drought: self.current_star5_drought,
                    longest_star4_streak: self.longest_star4_streak,
                    weapon: self.weapon,
                    weapon_star5: self.weapon_star5,
                    weapon_star4: self.weapon_star4,
                    character: self.character,
                    character_star5: self.character_star5,
                    character_star4: self.character_star4,
                    most_common_star5: self
                        .individual_star5_count
                        .iter()
                        .map(|x| x.1)
                        .max_by_key(|(count, _)| count)
                        .map(|(count, item)| (*item, *count)),
                    most_common_star4: self
                        .individual_star4_count
                        .iter()
                        .map(|x| x.1)
                        .max_by_key(|(count, _)| count)
                        .map(|(count, item)| (*item, *count)),
                }
            }
        }

        let numerical_stats: IntermediateStats =
            results.iter().fold(Default::default(), |stats, result| {
                stats.update(result, weapon_ident)
            });

        numerical_stats.to_final_stat()
    }

    fn print_to_console(&self, pool_name: &str) {
        println!(
            "你在{}中一共进行了{}抽，其中五星{}抽，四星{}抽，三星{}抽",
            style(pool_name).magenta().bold(),
            style(&self.total.to_string()).blue(),
            style(&self.star5.to_string()).yellow(),
            style(&self.star4.to_string()).magenta(),
            style(&self.star3.to_string()).blue(),
        );
        println!(
            "综合出率五星{}%，四星{:.2}%",
            style(&format!(
                "{:.2}",
                self.star5 as f64 / self.total as f64 * 100.0
            ))
            .yellow(),
            style(&format!(
                "{:.2}",
                self.star4 as f64 / self.total as f64 * 100.0
            ))
            .magenta(),
        );
        println!(
            "共抽出{}个武器，其中五星{}抽，四星{}抽",
            style(&self.weapon.to_string()).blue(),
            style(&self.weapon_star5.to_string()).yellow(),
            style(&self.weapon_star4.to_string()).magenta(),
        );
        println!(
            "共抽出{}个角色，其中五星{}抽，四星{}抽",
            style(&self.character.to_string()).blue(),
            style(&self.character_star5.to_string()).yellow(),
            style(&self.character_star4.to_string()).magenta(),
        );
        println!(
            "最多连续抽出{}个五星，连续抽出{}个四星",
            style(&self.longest_star5_streak.to_string()).yellow(),
            style(&self.longest_star4_streak.to_string()).magenta(),
        );
        println!(
            "最多{}抽未抽出五星，目前{}抽未抽出五星",
            style(&self.longest_star5_drought.to_string()).red(),
            style(&self.current_star5_drought.to_string()).red(),
        );
        if let Some((item, count)) = self.most_common_star5 {
            println!(
                "抽出的五星中，{}出现次数最多，抽出{}次",
                style(&item.name).yellow(),
                style(&count.to_string()).blue(),
            );
        }
        if let Some((item, count)) = self.most_common_star4 {
            println!(
                "抽出的四星中，{}出现次数最多，抽出{}次",
                style(&item.name).magenta(),
                style(&count.to_string()).blue(),
            );
        }
    }
}

fn export_results(
    results: &Vec<(&Item, DateTime<Local>)>,
    path: &Path,
    pb: ProgressBar,
) -> anyhow::Result<()> {
    pb.set_message("正在导出");
    let mut output = File::create(path)?;
    // UTF-8 BOM
    output.write_all(&[0xEF, 0xBB, 0xBF])?;
    writeln!(output, "抽卡时间,抽卡结果,类型,稀有度")?;
    pb.tick();
    for (item, time) in results.iter() {
        writeln!(
            output,
            "{},{},{},{}",
            time.format("%Y-%m-%d %T"),
            item.name,
            item.item_type,
            item.rank_type
        )?;
        pb.tick();
    }

    pb.finish_with_message("导出完毕");
    Ok(())
}

async fn run() -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use win32console::console::WinConsole;
        use win32console::structs::console_font_info_ex::ConsoleFontInfoEx;
        use win32console::structs::coord::Coord;
        WinConsole::set_input_code(65001).expect("unable to set console encoding");
        WinConsole::set_output_code(65001).expect("unable to set console encoding");
        let font_name_vec: Vec<u16> = "SimHei".encode_utf16().collect();
        let mut font_name = [0; 32];
        font_name[..font_name_vec.len()].clone_from_slice(&font_name_vec);
        let font = ConsoleFontInfoEx {
            size: std::mem::size_of::<ConsoleFontInfoEx>() as u32,
            font_index: 0,
            font_size: Coord { x: 14, y: 27 },
            font_family: 54,
            font_weight: 400,
            face_name: font_name,
        };
        WinConsole::output()
            .set_font_ex(font, false)
            .expect("unable to set console font");
    }

    let theme = if cfg!(windows) {
        ColorfulTheme {
            prompt_suffix: dialoguer::console::style(">".to_string())
                .for_stderr()
                .black()
                .bright(),
            active_item_prefix: dialoguer::console::style(">".to_string())
                .for_stderr()
                .green(),
            picked_item_prefix: dialoguer::console::style(">".to_string())
                .for_stderr()
                .green(),
            success_prefix: dialoguer::console::style("√".to_string())
                .for_stderr()
                .green(),
            error_prefix: dialoguer::console::style("×".to_string())
                .for_stderr()
                .red(),
            ..ColorfulTheme::default()
        }
    } else {
        ColorfulTheme::default()
    };

    let spinner_style = if cfg!(windows) {
        ProgressStyle::default_spinner().tick_chars("▁▃▅▇█▇▅▃▁√")
    } else {
        ProgressStyle::default_spinner()
    };

    let base_url: Url = Input::with_theme(&theme)
        .with_prompt("请输入网址")
        .validate_with(|input: &String| -> anyhow::Result<()> {
            let url = Url::parse(input).map_err(|err| anyhow!("输入不是网址: {}", err))?;
            let arg_map: HashMap<String, String> = url
                .query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect();
            if REQUIRED_FIELDS.iter().all(|&key| arg_map.contains_key(key)) {
                Ok(())
            } else {
                Err(anyhow!("输入网址不是有效的抽卡记录网址"))
            }
        })
        .interact()?
        .parse()
        .unwrap();

    let mut arg_map: HashMap<String, String> = base_url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();

    let retained_fields: HashSet<&'static str> = REQUIRED_FIELDS
        .iter()
        .cloned()
        .chain(ADDITIONAL_FIELDS.iter().cloned())
        .collect();

    arg_map.retain(|key, _| retained_fields.contains(key.as_str()));

    let client = Client::new();
    let mp = MultiProgress::new();
    let prepare_spinner_style = spinner_style
        .clone()
        .template("{prefix:.bold.dim} {spinner:.green} {wide_msg}");
    let gacha_config_pb =
        mp.add(ProgressBar::new_spinner().with_style(prepare_spinner_style.clone()));
    gacha_config_pb.enable_steady_tick(5);
    gacha_config_pb.set_prefix(&format!("[1/2]"));
    let item_list_pb = mp.add(ProgressBar::new_spinner().with_style(prepare_spinner_style));
    item_list_pb.enable_steady_tick(5);
    item_list_pb.set_prefix(&format!("[2/2]"));
    let progress_task = spawn_blocking(move || mp.join().unwrap());
    let (gacha_config, item_list, _) = tokio::join!(
        get_config_list(&client, &arg_map, gacha_config_pb),
        get_item_list(&client, &arg_map, item_list_pb),
        progress_task,
    );
    let gacha_config = gacha_config.context("加载卡池列表失败")?;
    let item_list = item_list.context("加载图鉴失败")?;
    let item_list: HashMap<usize, Item> = item_list
        .into_iter()
        .map(|item| (item.item_id.clone(), item))
        .collect();
    // 绝弦
    let weapon_ident = &item_list.get(&15405).unwrap().item_type;
    loop {
        let selection: usize = Select::with_theme(&theme)
            .with_prompt("请选择需要查询的卡池")
            .items(&gacha_config)
            .item("退出")
            .default(0)
            .interact()?;

        if selection == gacha_config.len() {
            break;
        }
        let config = &gacha_config[selection];
        let pb = ProgressBar::new_spinner().with_style(
            spinner_style
                .clone()
                .template("{spinner:.green} {msg}加载{pos}次抽卡记录"),
        );
        let mut results = get_gacha_result_all(&client, &arg_map, &config.key, pb).await?;
        results.reverse();
        let results = associate_items(&item_list, results);
        let stats = Statistics::from(&results, weapon_ident);

        stats.print_to_console(&config.name);

        if Confirm::with_theme(&theme)
            .with_prompt("是否导出抽卡记录")
            .wait_for_newline(true)
            .default(true)
            .interact()?
        {
            let mut save_path = current_dir().unwrap_or(PathBuf::new());
            let now = Local::now();
            save_path.push(format!(
                "{}-{}.csv",
                now.format("%Y-%m-%d %H-%M-%S"),
                config.name,
            ));
            let save_path = Input::with_theme(&theme)
                .with_prompt("保存位置")
                .validate_with(|path: &String| -> anyhow::Result<()> {
                    path.parse::<PathBuf>()?;
                    Ok(())
                })
                .default(save_path.display().to_string())
                .interact()?;
            let save_path = PathBuf::from(save_path).with_extension("csv");

            let pb = ProgressBar::new_spinner().with_style(
                spinner_style
                    .clone()
                    .template("{spinner:.green} {wide_msg}"),
            );
            export_results(&results, &save_path, pb)?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Err(err) = run().await {
        println!(
            "{}{}{}",
            style("错误: ").red(),
            err,
            err.source()
                .map(|err| format!(": {}", err))
                .unwrap_or("".to_owned())
        );
        Input::<String>::new()
            .with_prompt("按回车键退出")
            .allow_empty(true)
            .interact()?;
        Err(err)
    } else {
        Ok(())
    }
}
