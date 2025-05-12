use std::path::Path;
use tracing::Level;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{
    fmt::{self, format::FmtSpan, time::SystemTime},
    prelude::*,
    EnvFilter,
};

use crate::{errors::Error, options::LoggingOptions};

/// 初始化日志系统
/// 
/// # 参数
/// 
/// * `opts` - 程序配置选项
/// 
/// # 返回值
/// 
/// 返回初始化结果
pub fn init_logger(opts: &LoggingOptions) -> Result<(), Error> {
    // 创建日志目录
    let log_dir = opts.directory.clone();
    std::fs::create_dir_all(&log_dir)?;

    // 配置日志级别
    let level = match opts.level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    // 配置日志轮转
    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .max_log_files(opts.rotation.max_files as usize)
        .filename_prefix(&opts.file_name_pattern)
        .build(&log_dir)
        .map_err(|e| Error::Logger(e.to_string()))?;

    // 获取格式化选项
    let format_opts = opts.format.clone();

    // 创建订阅者
    let file_layer = fmt::layer()
        .with_file(format_opts.file)
        .with_line_number(format_opts.line_number)
        .with_thread_ids(format_opts.thread_id)
        .with_target(format_opts.target)
        .with_level(format_opts.level)
        .with_timer(SystemTime::default())
        .with_ansi(false)
        .with_writer(file_appender)
        .with_span_events(FmtSpan::FULL);

    // 创建环境过滤器
    let env_filter = EnvFilter::from_default_env()
        .add_directive(level.into());

    // 设置全局默认订阅者
    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .try_init()
        .map_err(|e| Error::Logger(e.to_string()))?;

    // 如果启用了压缩，添加压缩处理
    if opts.rotation.compress {
        let compress_dir = log_dir.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600)); // 每小时检查一次
                compress_old_logs(Path::new(&compress_dir));
            }
        });
    }

    Ok(())
}

/// 压缩旧的日志文件
fn compress_old_logs(log_dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(log_dir) {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "log" && !path.with_extension("gz").exists() {
                    if let Err(e) = compress_file(&path) {
                        eprintln!("Failed to compress log file {:?}: {}", path, e);
                    }
                }
            }
        }
    }
}

/// 压缩单个文件
fn compress_file(path: &Path) -> std::io::Result<()> {
    use std::fs::File;
    use std::io::{Read, Write};
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let mut input = File::open(path)?;
    let output = File::create(path.with_extension("gz"))?;
    let mut encoder = GzEncoder::new(output, Compression::default());
    
    let mut buffer = Vec::new();
    input.read_to_end(&mut buffer)?;
    encoder.write_all(&buffer)?;
    encoder.finish()?;
    
    std::fs::remove_file(path)?;
    
    Ok(())
} 