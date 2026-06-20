use std::time::Duration;

use futures_util::Stream;
use poem::listener::{RustlsCertificate, RustlsConfig};
use rcgen::{Certificate, CertificateParams, KeyPair, RcgenError, SanType};

fn create_certificate() -> Result<(String, String), RcgenError> {
    // 运行时读取证书文件
    let base_path = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let ca_cert_path = base_path.join("cert/ca.crt");
    let ca_key_path = base_path.join("cert/ca.key");

    let ca_cert_pem = std::fs::read_to_string(&ca_cert_path)
        .unwrap_or_else(|_| panic!("Failed to read CA certificate from {:?}", ca_cert_path));
    let ca_key_pem = std::fs::read_to_string(&ca_key_path)
        .unwrap_or_else(|_| panic!("Failed to read CA key from {:?}", ca_key_path));

    let key = KeyPair::from_pem(&ca_key_pem)?;
    let params = CertificateParams::from_ca_cert_pem(&ca_cert_pem, key)?;
    let ca_cert = Certificate::from_params(params)?;

    let mut params = CertificateParams::default();
    params
        .subject_alt_names
        .push(SanType::IpAddress("127.0.0.1".parse().unwrap()));
    params
        .subject_alt_names
        .push(SanType::IpAddress("0.0.0.0".parse().unwrap()));
    params
        .subject_alt_names
        .push(SanType::DnsName("localhost".to_string()));
    let gen_cert = Certificate::from_params(params)?;

    let server_crt = gen_cert.serialize_pem_with_signer(&ca_cert)?;
    let server_key = gen_cert.serialize_private_key_pem();

    Ok((server_crt, server_key))
}

pub fn create_self_signed_config() -> impl Stream<Item = RustlsConfig> {
    async_stream::stream! {
        loop {
            if let Ok((cert, key)) = create_certificate() {
                yield RustlsConfig::new().fallback(RustlsCertificate::new().cert(cert).key(key));
            }
            tokio::time::sleep(Duration::from_secs(60 * 5)).await;
        }
    }
}
