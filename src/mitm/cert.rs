use std::{
    fs::{read, File},
    io::Write,
    path::PathBuf,
};

use anyhow::Context;
use console::style;
use indicatif::ProgressBar;
use rcgen::{
    generate_simple_self_signed, BasicConstraints, Certificate as GenCertificate,
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
};
use rustls::{Certificate, PrivateKey};

use crate::{mitm::DOMAIN_INTERCEPT, style::SPINNER_STYLE};

pub const CERT_FILENAME: &str = "ca.cer";
const KEY_FILENAME: &str = "ca.key";

/// Set up the certificate to intercept traffic. This will first look for `CERT_FILENAME`
/// and `KEY_FILENAME` in the current directory and use the file as-is as the root CA certificate
/// if they exist. Otherwise new CA certificate/key will be generated and exported.
/// A certificate specifically for the website will then be signed by the CA
pub fn setup_certificate() -> anyhow::Result<(Certificate, PrivateKey)> {
    let cert_path: PathBuf = [".", CERT_FILENAME].iter().collect();
    let key_path: PathBuf = [".", KEY_FILENAME].iter().collect();

    let ca_cert = if cert_path.exists() && key_path.exists() {
        let pb = ProgressBar::new_spinner().with_style(
            SPINNER_STYLE
                .clone()
                .template("{spinner:.green} {wide_msg}"),
        );
        pb.set_message("读取已保存的自签发根证书及私钥");
        pb.enable_steady_tick(5);

        let cert_der = read(&cert_path)
            .with_context(|| format!("无法读取证书文件 {}", style(CERT_FILENAME).dim()))?;
        let key_der = read(&key_path)
            .with_context(|| format!("无法读取私钥文件 {}", style(KEY_FILENAME).dim()))?;

        let key_pair = KeyPair::from_der(&key_der).context("无效的证书私钥")?;
        let params =
            CertificateParams::from_ca_cert_der(&cert_der, key_pair).context("无效的根证书")?;
        pb.finish_with_message("已加载自签发根证书及私钥");

        GenCertificate::from_params(params).context("无效的根证书")?
    } else {
        let pb = ProgressBar::new_spinner().with_style(
            SPINNER_STYLE
                .clone()
                .template("{spinner:.green} {wide_msg}"),
        );
        pb.set_message("生成自签发根证书及私钥");
        pb.enable_steady_tick(5);
        let params = generate_ca_cerficate_params();
        let cert = GenCertificate::from_params(params).context("无法生成自签发证书")?;
        pb.set_message("保存自签发证书及私钥");
        let cert_der = cert.serialize_der().context("无法导出根证书")?;
        let key_der = cert.serialize_private_key_der();

        let mut cert_file = File::create(&cert_path).context("无法创建证书文件")?;
        cert_file.write_all(&cert_der).context("无法写入证书")?;
        cert_file.sync_all().context("无法写入证书")?;
        drop(cert_file);

        let mut key_file = File::create(&key_path).context("无法创建私钥文件")?;
        key_file.write_all(&key_der).context("无法写入私钥")?;
        key_file.sync_all().context("无法写入私钥")?;
        drop(key_file);
        pb.finish_with_message(&format!(
            "已保存生成的自签发根证书到 {}，私钥到 {}",
            style(CERT_FILENAME).dim(),
            style(KEY_FILENAME).dim()
        ));

        println!(
            "{} 请将证书 {} 加入系统的根证书信任库中",
            style("[提醒]").green(),
            style(CERT_FILENAME).dim()
        );
        println!("{} 证书私钥泄露可能会导致安全问题", style("[警告]").red());

        cert
    };

    let cert = generate_simple_self_signed(
        DOMAIN_INTERCEPT
            .iter()
            .cloned()
            .map(ToOwned::to_owned)
            .collect::<Vec<String>>(),
    )
    .context("无法生成网站用证书")?;
    let cert_der = cert
        .serialize_der_with_signer(&ca_cert)
        .context("无法签发网站用证书")?;
    let key_der = cert.serialize_private_key_der();

    Ok((Certificate(cert_der), PrivateKey(key_der)))
}

/// Generate certificate parameters for root CA certificate
fn generate_ca_cerficate_params() -> CertificateParams {
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, "DO_NOT_TRUST Genshin Exporter CA");
    // TODO: fork `rcgen` and add support for [Key Usage Extension](https://tools.ietf.org/html/rfc5280#section-4.2.1.3)
    let mut params = CertificateParams::new(
        DOMAIN_INTERCEPT
            .iter()
            .cloned()
            .map(ToOwned::to_owned)
            .collect::<Vec<String>>(),
    );
    params.distinguished_name = distinguished_name;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ServerAuth);
    params
}
