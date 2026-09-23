//! Property-local TLS for aice services.
//!
//! Each property gets its own certificate authority (`ca.pem` / `ca.key`) that
//! signs the facilitator's and backend's server certificates. Pods pin the CA,
//! so no public CA or internet access is needed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

pub use tokio_rustls::{TlsAcceptor, TlsConnector};

const CA_CERT: &str = "ca.pem";
const CA_KEY: &str = "ca.key";
const SERVER_CERT: &str = "server.pem";
const SERVER_KEY: &str = "server.key";
const CA_COMMON_NAME: &str = "aice property CA";

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("io {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("certificate generation: {0}")]
    Generate(#[from] rcgen::Error),
    #[error("pem {path}: {message}")]
    Pem { path: PathBuf, message: String },
    #[error("tls config: {0}")]
    Config(#[from] rustls::Error),
}

/// Paths of the property CA.
#[derive(Clone, Debug)]
pub struct CaFiles {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> TlsError + '_ {
    move |source| TlsError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn ca_params() -> Result<CertificateParams, TlsError> {
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, CA_COMMON_NAME);
    params.distinguished_name = name;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let now = OffsetDateTime::now_utc();
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(3650);
    Ok(params)
}

/// Create the property CA in `dir` unless it already exists.
pub fn ensure_ca(dir: &Path) -> Result<CaFiles, TlsError> {
    let files = CaFiles {
        cert_path: dir.join(CA_CERT),
        key_path: dir.join(CA_KEY),
    };
    if files.cert_path.exists() && files.key_path.exists() {
        return Ok(files);
    }
    std::fs::create_dir_all(dir).map_err(io_error(dir))?;
    let key = KeyPair::generate()?;
    let cert = ca_params()?.self_signed(&key)?;
    write_secret(&files.key_path, &key.serialize_pem())?;
    std::fs::write(&files.cert_path, cert.pem()).map_err(io_error(&files.cert_path))?;
    Ok(files)
}

/// Rebuild a signing handle for the CA from its key. The issuer name and key
/// match the stored `ca.pem`, so certificates it signs chain to that file.
fn load_ca(files: &CaFiles) -> Result<(Certificate, KeyPair), TlsError> {
    let pem = std::fs::read_to_string(&files.key_path).map_err(io_error(&files.key_path))?;
    let key = KeyPair::from_pem(&pem)?;
    let cert = ca_params()?.self_signed(&key)?;
    Ok((cert, key))
}

/// Issue a server certificate for `hostnames` (DNS names or IP addresses).
pub fn issue_server_cert(
    ca: &CaFiles,
    hostnames: &[String],
    cert_out: &Path,
    key_out: &Path,
) -> Result<(), TlsError> {
    let (ca_cert, ca_key) = load_ca(ca)?;
    let mut params = CertificateParams::new(hostnames.to_vec())?;
    let mut name = DistinguishedName::new();
    name.push(
        DnType::CommonName,
        hostnames.first().map(String::as_str).unwrap_or("aice"),
    );
    params.distinguished_name = name;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let now = OffsetDateTime::now_utc();
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(730);
    let key = KeyPair::generate()?;
    let cert = params.signed_by(&key, &ca_cert, &ca_key)?;
    if let Some(parent) = cert_out.parent() {
        std::fs::create_dir_all(parent).map_err(io_error(parent))?;
    }
    write_secret(key_out, &key.serialize_pem())?;
    std::fs::write(cert_out, cert.pem()).map_err(io_error(cert_out))?;
    Ok(())
}

/// CA plus a server certificate in `dir`, created on first use.
pub fn ensure_server_cert(
    dir: &Path,
    hostnames: &[String],
) -> Result<(PathBuf, PathBuf), TlsError> {
    let ca = ensure_ca(dir)?;
    let cert = dir.join(SERVER_CERT);
    let key = dir.join(SERVER_KEY);
    if !(cert.exists() && key.exists()) {
        issue_server_cert(&ca, hostnames, &cert, &key)?;
    }
    Ok((cert, key))
}

fn read_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let bytes = std::fs::read(path).map_err(io_error(path))?;
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&bytes)
        .collect::<Result<_, _>>()
        .map_err(|error| TlsError::Pem {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    if certs.is_empty() {
        return Err(TlsError::Pem {
            path: path.to_path_buf(),
            message: "no certificates".to_string(),
        });
    }
    Ok(certs)
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// rustls server config for a PEM certificate chain and private key.
pub fn server_config(cert: &Path, key: &Path) -> Result<Arc<rustls::ServerConfig>, TlsError> {
    let certs = read_certs(cert)?;
    let key_bytes = std::fs::read(key).map_err(io_error(key))?;
    let key_der = PrivateKeyDer::from_pem_slice(&key_bytes).map_err(|error| TlsError::Pem {
        path: key.to_path_buf(),
        message: error.to_string(),
    })?;
    let mut config = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(certs, key_der)?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

pub fn acceptor(cert: &Path, key: &Path) -> Result<TlsAcceptor, TlsError> {
    Ok(TlsAcceptor::from(server_config(cert, key)?))
}

/// rustls client config that trusts only the CA in `ca_pem`.
pub fn client_config_trusting(ca_pem: &Path) -> Result<Arc<rustls::ClientConfig>, TlsError> {
    let mut roots = rustls::RootCertStore::empty();
    for cert in read_certs(ca_pem)? {
        roots.add(cert)?;
    }
    let config = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// SHA-256 of the first certificate's DER, as colon-separated hex.
/// Pods pin this value for the property CA.
pub fn fingerprint_sha256(cert: &Path) -> Result<String, TlsError> {
    let certs = read_certs(cert)?;
    let digest = Sha256::digest(certs[0].as_ref());
    Ok(hex::encode_upper(digest)
        .as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).into_owned())
        .collect::<Vec<_>>()
        .join(":"))
}

fn write_secret(path: &Path, contents: &str) -> Result<(), TlsError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(io_error(path))?;
    std::io::Write::write_all(&mut file, contents.as_bytes()).map_err(io_error(path))
}
