/// Client for Genshin API
use std::{collections::HashMap, future, iter::once};

use anyhow::{anyhow, Context};
use chrono::{Local, TimeZone};
use futures::stream::{self, StreamExt, TryStreamExt};
use indicatif::{MultiProgress, ProgressBar};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT_LANGUAGE, UPGRADE_INSECURE_REQUESTS},
    Client as ReqClient, Url,
};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use tokio::task::spawn_blocking;

use crate::{
    data_type::{Item, ItemType, Pool, Pull, Rarity},
    mitm::PAGE_INTERCEPT_SUFFIX,
    style::SPINNER_STYLE,
};

/// Return the url for item list given region of server and language to use
fn item_list_url(region: &str, lang: &str) -> Url {
    Url::parse(&format!(
        "https://webstatic-sea.mihoyo.com/hk4e/gacha_info/{}/items/{}.json",
        region, lang
    ))
    .unwrap()
}

/// ID for "The Stringless", used to identify the local identifier for weapon
const WEAPON_ID: &str = "15405";
/// ID for "Venti", used to identify the local identifier for character
const CHARACTER_ID: &str = "1022";

/// The user-agent to use
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 6.1; Unity 3D; ZFBrowser 2.1.0; Genshin Impact 1.2.0_1565149_1627898) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/72.0.3626.96 Safari/537.36";

/// A generic API response
#[derive(Debug, Serialize, Deserialize)]
struct ApiResponse<T> {
    retcode: i32,
    message: String,
    data: Option<T>,
}

/// Information of a pool
#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
struct GachaConfig {
    #[serde_as(as = "DisplayFromStr")]
    id: usize,
    key: String,
    name: String,
}

impl Into<Pool> for GachaConfig {
    fn into(self) -> Pool {
        Pool {
            id: self.id,
            key: self.key,
            name: self.name,
        }
    }
}

/// Payload for endpoint `getConfigList`
#[derive(Debug, Serialize, Deserialize)]
struct ConfigListData {
    gacha_type_list: Vec<GachaConfig>,
    region: String,
}

/// Information for a single pull
#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
struct GachaResult {
    #[serde_as(as = "DisplayFromStr")]
    uid: usize,
    gacha_type: String,
    count: String,
    time: String,
    #[serde(flatten)]
    item: GachaItem,
    lang: String,
}

/// Payload for endpoint `getGachaLog`
/// Currently it seems that `page`, `size`, `total` are unused
#[derive(Debug, Serialize, Deserialize)]
struct GachaResultPage {
    page: String,
    size: String,
    total: String,
    list: Vec<GachaResult>,
    region: String,
}

/// Payload for [`item_list_url`]
#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
struct GachaItem {
    item_id: String,
    name: String,
    item_type: String,
    #[serde_as(as = "DisplayFromStr")]
    rank_type: u8,
}

/// A client used to query Genshin gacha info
#[derive(Debug)]
pub struct Client {
    /// identifier for a weapon
    weapon_identifier: String,
    /// identifier for a character
    character_identifier: String,
    /// metadata for pools
    pools: Vec<Pool>,
    /// backing http client
    client: ReqClient,
    /// base query to use
    base_query: BaseQuery,
    /// base url to use
    base_url: String,
}

impl Client {
    /// Create the client from an url pointing to in-game gacha page
    pub async fn new(url: Url) -> anyhow::Result<Self> {
        let base_query = BaseQuery::new(&url)?;

        let base_url = format!(
            "{}://{}{}",
            url.scheme(),
            url.host_str().unwrap(),
            url.path().replacen(PAGE_INTERCEPT_SUFFIX, "", 1)
        );

        // build web client
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
        headers.insert(UPGRADE_INSECURE_REQUESTS, HeaderValue::from_static("1"));
        let client = ReqClient::builder()
            .default_headers(headers)
            .user_agent(USER_AGENT)
            .gzip(true)
            .no_proxy()
            .build()
            .unwrap();

        // acquire information of pools and items
        let mp = MultiProgress::new();
        let pools_pb = Self::add_spinner(&mp, 1, 2);
        let items_pb = Self::add_spinner(&mp, 2, 2);

        let pools_task = Self::request_pools(&client, &base_query, &base_url, pools_pb);
        let items_task = Self::request_items(&client, &base_query, items_pb);
        let progress_task = spawn_blocking(move || mp.join());
        let (pools, identifiers, _) = tokio::join!(pools_task, items_task, progress_task);
        let pools = pools.context("加载卡池列表失败")?;
        let (weapon_identifier, character_identifier) = identifiers.context("加载图鉴失败")?;

        Ok(Self {
            weapon_identifier,
            character_identifier,
            pools,
            client,
            base_query,
            base_url,
        })
    }

    /// Get information of all the pools
    pub fn get_pools(&self) -> &Vec<Pool> {
        &self.pools
    }

    /// Get a chronological log of all the pulls from `pool`
    pub async fn request_gacha_log(&self, pool: &Pool) -> anyhow::Result<Vec<Pull>> {
        // set up additional queries
        let query = vec![
            ("init_type".to_owned(), pool.key.clone()),
            ("gacha_type".to_owned(), pool.key.clone()),
            ("size".to_owned(), "20".to_owned()),
        ];
        // set up a progress bar
        let pb = ProgressBar::new_spinner().with_style(
            SPINNER_STYLE
                .clone()
                .template("{spinner:.green} {msg}加载{pos}次抽卡记录"),
        );
        pb.set_message("正在加载，已");
        // iterate through pages
        let mut pull_list: Vec<Pull> = stream::iter(1..)
            .then(|page: usize| {
                let query = query.clone();
                async move {
                    // get records from current page
                    let page: GachaResultPage = Self::issue_api(
                        &self.client,
                        &self.base_query,
                        &format!("{}/getGachaLog", self.base_url),
                        query
                            .into_iter()
                            .chain(once(("page".to_owned(), page.to_string()))),
                    )
                    .await?;
                    // convert each pull from API format to our format
                    let page: Vec<Pull> = page
                        .list
                        .into_iter()
                        .map(|pull| Pull {
                            time: Local.datetime_from_str(&pull.time, "%Y-%m-%d %T").unwrap(),
                            item: {
                                let rarity = match pull.item.rank_type {
                                    5 => Rarity::Five,
                                    4 => Rarity::Four,
                                    3 => Rarity::Three,
                                    _ => unreachable!("图鉴中含有范围外的稀有度"),
                                };
                                let item_type = if pull.item.item_type == self.weapon_identifier {
                                    ItemType::Weapon
                                } else {
                                    ItemType::Character
                                };
                                Item {
                                    name: pull.item.name,
                                    rarity,
                                    item_type,
                                }
                            },
                        })
                        .collect();
                    Ok::<_, anyhow::Error>(page)
                }
            })
            // stop when a page is empty, indicating end of log
            .try_take_while(|page| future::ready(Ok(!page.is_empty())))
            .and_then(|page| {
                // update the progress bar
                pb.inc(page.len() as u64);
                future::ready(Ok(stream::iter(
                    page.into_iter().map(Ok::<_, anyhow::Error>),
                )))
            })
            // flatten all the pages into one big iterator
            .try_flatten()
            .try_collect()
            .await?;
        // reverse the list so that the log is chronological
        pull_list.reverse();
        // finish the progress bar
        pb.set_length(pull_list.len() as u64);
        pb.finish_with_message("已");
        Ok(pull_list)
    }

    /// Verify whether a url pointing to a gacha page contains proper query compoenent
    pub fn verify_url(url: &Url) -> bool {
        BaseQuery::new(url).is_ok()
    }

    /// Add a spinner to a multi progress bar
    fn add_spinner(mp: &MultiProgress, step: usize, total: usize) -> ProgressBar {
        let style = SPINNER_STYLE
            .clone()
            .template("{prefix:.bold.dim}{spinner:.green} {msg}");
        let pb = mp.add(ProgressBar::new_spinner().with_style(style));
        pb.enable_steady_tick(5);
        pb.set_prefix(&format!("[{}/{}]", step, total));
        pb
    }

    /// Get a list of pools that can be queried
    async fn request_pools(
        client: &ReqClient,
        base_query: &BaseQuery,
        base_url: &str,
        pb: ProgressBar,
    ) -> anyhow::Result<Vec<Pool>> {
        pb.set_message("加载卡池列表");
        let config_list: ConfigListData = Self::issue_api(
            client,
            base_query,
            &format!("{}/getConfigList", base_url),
            None,
        )
        .await?;
        pb.finish_with_message("已加载卡池列表");
        Ok(config_list
            .gacha_type_list
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Get the identifier for weapon and character
    async fn request_items(
        client: &ReqClient,
        base_query: &BaseQuery,
        pb: ProgressBar,
    ) -> anyhow::Result<(String, String)> {
        pb.set_message("加载图鉴");
        // get region/lang specific url
        let url = item_list_url(&base_query.region, &base_query.lang);
        let item_list = client
            .get(url)
            .send()
            .await?
            .json::<Vec<GachaItem>>()
            .await?;
        let weapon_identifier = item_list
            .iter()
            .find(|item| item.item_id == WEAPON_ID)
            .ok_or(anyhow!("内置的绝弦ID已过期，无法建立图鉴"))?
            .item_type
            .clone();
        let character_identifier = item_list
            .iter()
            .find(|item| item.item_id == CHARACTER_ID)
            .ok_or(anyhow!("内置的温迪ID已过期，无法建立图鉴"))?
            .item_type
            .clone();
        pb.finish_with_message("已加载图鉴");
        Ok((weapon_identifier, character_identifier))
    }

    /// Get response from Genshin API server
    async fn issue_api<T, Q, K, V>(
        client: &ReqClient,
        base_query: &BaseQuery,
        endpoint: &str,
        additional_query: Q,
    ) -> anyhow::Result<T>
    where
        for<'a> T: Deserialize<'a>,
        HashMap<String, String>: Extend<(K, V)>,
        Q: IntoIterator<Item = (K, V)>,
    {
        // build query component
        let mut query = base_query.as_hashmap();
        query.extend(additional_query.into_iter());
        let url = Url::parse_with_params(endpoint, query).unwrap();
        let resp = client
            .get(url)
            .send()
            .await?
            .json::<ApiResponse<T>>()
            .await?;
        if resp.retcode != 0 {
            Err(anyhow!(resp.message))
        } else {
            Ok(resp.data.unwrap())
        }
    }
}

/// The base query extracted from in-game url to the banner page
#[derive(Debug)]
struct BaseQuery {
    // Required fields
    authkey_ver: String,
    sign_type: String,
    auth_appid: String,
    gacha_id: String,
    lang: String,
    game_biz: String,
    authkey: String,
    region: String,

    // Optional fields
    device_type: Option<String>,
    ext: Option<String>,
    game_version: Option<String>,
}

/// Extract a value from a hashmap and propagate not found error accordingly
macro_rules! value_from_hashmap {
    ($map: expr, !$field: ident) => {
        value_from_hashmap!($map, ?$field).ok_or_else(|| {
            anyhow!(format!(
                "expected field {} not found in the query",
                stringify!($field)
            ))
        })?
    };
    ($map: expr, ?$field: ident) => {
        $map.get(stringify!($field)).map(Clone::clone)
    };
}
macro_rules! from_hashmap {
    ($item: tt, $map: expr; $($mark: tt$field: ident)+) => {
        $item {
            $($field: value_from_hashmap!($map, $mark$field),)+
        }
    };
}

/// Insert a (field_name, field_value) pair into a hashmap
macro_rules! insert_to_hashmap {
    ($item: expr, $map: expr, !$field: ident) => {
        $map.insert(stringify!($field).to_owned(), $item.$field.clone());
    };
    ($item: expr, $map: expr, ?$field: ident) => {
        if let Some(v) = &$item.$field {
            $map.insert(stringify!($field).to_owned(), v.clone());
        }
    };
}

impl BaseQuery {
    fn new(url: &Url) -> anyhow::Result<Self> {
        let map: HashMap<String, String> = url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();

        Ok(from_hashmap!(Self, map;
            !authkey_ver !sign_type !auth_appid !gacha_id !lang
            !game_biz !authkey !region
            ?device_type ?ext ?game_version))
    }

    fn as_hashmap(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        insert_to_hashmap!(self, map, !authkey_ver);
        insert_to_hashmap!(self, map, !sign_type);
        insert_to_hashmap!(self, map, !auth_appid);
        insert_to_hashmap!(self, map, !gacha_id);
        insert_to_hashmap!(self, map, !lang);
        insert_to_hashmap!(self, map, !game_biz);
        insert_to_hashmap!(self, map, !authkey);
        insert_to_hashmap!(self, map, !region);

        insert_to_hashmap!(self, map, ?device_type);
        insert_to_hashmap!(self, map, ?ext);
        insert_to_hashmap!(self, map, ?game_version);

        map
    }
}
