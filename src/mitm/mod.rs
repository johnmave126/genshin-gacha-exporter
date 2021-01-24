pub mod cert;
pub mod service;

use anyhow::anyhow;
use reqwest::Url;
use tokio::sync::oneshot;

use dialoguer::Confirm;
use indicatif::ProgressBar;

use crate::{
    mitm::{cert::setup_certificate, service::make_mitm_server},
    style::{SPINNER_STYLE, THEME},
};

pub const DOMAIN_INTERCEPT: &[&str] = &["hk4e-api.mihoyo.com", "hk4e-api-os.mihoyo.com"];
pub const PAGE_INTERCEPT_SUFFIX: &str = "getGachaLog";

/// Set up proxy server to tap connection and look for gacha url
pub async fn tap_for_url() -> anyhow::Result<Url> {
    let (certificate, private_key) = setup_certificate()?;
    let (mut receiver, server) = make_mitm_server(certificate, private_key);
    let server_addr = server.local_addr();

    #[cfg(target_os = "windows")]
    let old_proxy_settings = {
        // Under windows, ask user whether system proxy should be automatically set
        if Confirm::with_theme(&*THEME)
            .with_prompt("是否自动配置系统HTTP代理")
            .wait_for_newline(true)
            .default(true)
            .interact()?
        {
            use proxyconf::internet_settings::modern::{
                empty_config,
                registry::{self, get_current_user_location},
            };
            use std::process::Command;

            // add certificate to user root store
            Command::new("certutil")
                .args(&["-user", "-addstore", "root", cert::CERT_FILENAME])
                .output()
                .ok();

            let mut proxy_config = empty_config();
            proxy_config.use_manual_proxy = true;
            proxy_config.manual_proxy_address = format!("127.0.0.1:{}", server_addr.port());
            proxy_config.manual_proxy_bypass_list = "*.local".to_owned();
            let proxy_location = get_current_user_location();

            let old_proxy_settings =
                registry::read_full(&proxy_location).map_err(|e| anyhow!(format!("{}", e)))?;

            registry::write(&proxy_location, proxy_config)
                .map_err(|e| anyhow!(format!("{}", e)))?;

            Some(old_proxy_settings)
        } else {
            None
        }
    };
    let pb = ProgressBar::new_spinner()
        .with_style(SPINNER_STYLE.clone().template("{spinner:.green} {msg}"));
    pb.enable_steady_tick(5);
    pb.set_message(&format!(
        "HTTP代理已部署在 {}，正在等待检测抽卡页面",
        server_addr
    ));

    // Spin up the proxy server
    let (final_sender, final_receiver) = oneshot::channel();
    let server = server.with_graceful_shutdown(async move {
        let url = receiver.recv().await;
        final_sender.send(url).ok();
    });

    server.await?;
    let url = final_receiver
        .await?
        .ok_or_else(|| anyhow!("broken pipe of URL retrieval"))?;

    #[cfg(target_os = "windows")]
    if let Some(old_proxy_settings) = old_proxy_settings {
        // Restore proxy settings
        use proxyconf::internet_settings::modern::registry::{self, get_current_user_location};

        let proxy_location = get_current_user_location();
        registry::write_full(&proxy_location, &old_proxy_settings)
            .map_err(|e| anyhow!(format!("{}", e)))?;
    }

    pb.finish_with_message(&format!("成功获取抽卡页面： {}", url));

    Ok(url)
}
