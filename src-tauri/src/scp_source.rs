use super::*;

pub(super) fn ensure_scp_source_has_not_grown(source: &mut fs::File) -> Result<(), String> {
    let mut extra = [0_u8; 1];
    match source.read(&mut extra) {
        Ok(0) => Ok(()),
        Ok(_) => Err("SCP 本地源文件在传输中增长，已保留断点文件且未提升目标文件".to_string()),
        Err(error) => Err(format!("SCP 检查本地源文件结尾失败: {error}")),
    }
}

pub(super) fn scp_source_prefix_sha256(
    source: &mut fs::File,
    length: u64,
    progress: &TransferProgressContext,
) -> Result<String, String> {
    source
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|error| format!("SCP 定位本地前缀失败: {error}"))?;
    let mut digest = Sha256::new();
    let mut remaining = length;
    let mut buffer = vec![0_u8; 64 * 1024];
    while remaining > 0 {
        progress.check_cancelled()?;
        let take = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let read = source
            .read(&mut buffer[..take])
            .map_err(|error| format!("读取 SCP 本地前缀失败: {error}"))?;
        if read == 0 {
            return Err(format!(
                "SCP 本地源文件在校验续传前缀时提前结束（剩余 {remaining} 字节）"
            ));
        }
        digest.update(&buffer[..read]);
        remaining -= read as u64;
    }
    Ok(format!("{:x}", digest.finalize()))
}
