use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

const MAX_LANGUAGE_BYTES: u64 = 1024 * 1024;

pub fn install_github_language(
    locale: &str,
    repo: &str,
    reference: &str,
    output_dir: impl AsRef<Path>,
    expected_sha256: Option<&str>,
) -> Result<PathBuf> {
    validate_expected_sha256(expected_sha256)?;
    let url = cleanr_i18n::github_raw_language_url(repo, reference, locale)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .context("failed to create language download client")?;
    let response = client
        .get(&url)
        .send()
        .with_context(|| format!("failed to GET {url}"))?;
    if !response.status().is_success() {
        bail!("failed to download {url}: HTTP {}", response.status());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_LANGUAGE_BYTES)
    {
        bail!("language file exceeds the 1 MiB size limit");
    }
    let mut body = Vec::new();
    response
        .take(MAX_LANGUAGE_BYTES + 1)
        .read_to_end(&mut body)
        .with_context(|| format!("failed to read response body from {url}"))?;
    if body.len() as u64 > MAX_LANGUAGE_BYTES {
        bail!("language file exceeds the 1 MiB size limit");
    }
    let body = String::from_utf8(body).context("language file is not valid UTF-8")?;
    cleanr_i18n::validate_language_yaml(&body)
        .with_context(|| format!("downloaded language file {url} is invalid"))?;
    if let Some(expected) = expected_sha256 {
        let actual = format!("{:x}", Sha256::digest(body.as_bytes()));
        if !actual.eq_ignore_ascii_case(expected) {
            bail!("language file SHA-256 mismatch: expected {expected}, got {actual}");
        }
    }

    let file_name = url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .context("language download URL has no file name")?;
    let output_dir = output_dir.as_ref();
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    let path = output_dir.join(file_name);
    atomic_write(&path, body.as_bytes())?;
    Ok(path)
}

fn validate_expected_sha256(expected_sha256: Option<&str>) -> Result<()> {
    if let Some(expected) = expected_sha256
        && (expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        bail!("expected language SHA-256 must contain exactly 64 hexadecimal characters");
    }
    Ok(())
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(directory)
        .with_context(|| format!("failed to create temporary file in {}", directory.display()))?;
    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_sha256_before_downloading() {
        let error =
            install_github_language("en-US", "owner/repo", "main", ".", Some("not-a-sha256"))
                .expect_err("invalid hash must fail");
        assert!(error.to_string().contains("64 hexadecimal"));
    }
}
