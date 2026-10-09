#[cfg(all(test, unix))]
pub(super) fn write_local_transfer_file(path: &str, data: &[u8]) -> Result<(), String> {
    let mut output = PendingLocalTransferOutput::create(Path::new(path), "本地传输目标路径")?;
    output
        .file_mut()?
        .write_all(data)
        .map_err(|error| format!("写入本地文件失败: {error}"))?;
    output.finish()
}
