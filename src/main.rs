use clap::Parser;
use filesize::file_real_size_fast;
use futures;
use jwalk::WalkDir;
use num_cpus;
use std::fs;
use std::io::{stderr, stdout, Write};
use tokio;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(
    version = "0.0.2",
    about = "磁盘扫描工具",
    long_about = "高性能扫描文件夹下所有文件的总占用"
)]
struct Args {
    /// 输入需要扫描的文件，必须
    #[arg(short, long, required = true)]
    root: String,

    /// 输入需要排查的文件或者文件夹,多个用逗号隔开
    #[arg(short, long, value_delimiter = ',')]
    execlude: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let start = std::time::Instant::now();
    let args: Args = Args::parse();
    let mut tasks = vec![];
    for entry in fs::read_dir(args.root)? {
        let entry = entry?;
        if args
            .execlude
            .contains(&entry.file_name().to_str().unwrap().to_string())
        {
            writeln!(
                stderr(),
                "\x1b[41m execlude file or dir:{}\x1b[0m",
                entry.path().display()
            );
            stderr().flush();
            continue;
        }
        let task = tokio::spawn(async move {
            let total_filesize = WalkDir::new(entry.path())
                .follow_links(true)
                .min_depth(0)
                .sort(false)
                .skip_hidden(false)
                .parallelism(jwalk::Parallelism::RayonExistingPool {
                    pool: jwalk::rayon::ThreadPoolBuilder::new()
                        .stack_size(128 * 1024)
                        .num_threads(num_cpus::get())
                        .thread_name(|idx| format!("du-jwalk-{idx}"))
                        .build()
                        .expect("fields we set cannot fail")
                        .into(),
                    busy_timeout: None,
                })
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|entry| entry.file_type().is_file())
                .map(|e| {
                    e.metadata()
                        .and_then(|m| file_real_size_fast(e.path & m))
                        .unwrap_or(0)
                })
                .sum::<u64>();

            writeln!(
                stdout(),
                "\x1b[32m{:>10},\x1b[36m{}\x1b[0m",
                humansize::format_size(total_filesize, humansize::DECIMAL),
                entry.path().display()
            );
            stdout().flush();
        });
        tasks.push(task);
    }
    futures::future::join_all(tasks).await;
    writeln!(stdout(),"total time:{}", humantime::format_duration(start.elapsed()));
    Ok(())
}
