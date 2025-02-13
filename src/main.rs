use clap::Parser;
use filesize::file_real_size_fast;
use std::fs::{read_dir};
use std::io::{stderr, stdout, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time;
use serde::Serialize;

type WalkDir = jwalk::WalkDirGeneric<((), Option<Result<std::fs::Metadata, jwalk::Error>>)>;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(
    version = "1.0.1",
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

    /// csv结果文件保存路径
    #[arg[short, long]]
    output: Option<PathBuf>,
}

#[allow(non_snake_case)]
#[derive(Debug, Serialize)]
struct ShowInfo {
    #[allow(non_snake_case)]
    total_filesize_string: String,
    #[allow(non_snake_case)]
    filesSize_type: String,
    #[allow(non_snake_case)]
    path: String,
}

fn iter_from_path(root: &Path) -> WalkDir {
    WalkDir::new(root)
        .follow_links(false)
        .min_depth(0)
        .sort(false)
        .skip_hidden(false)
        .process_read_dir({
            move |_, _, _, dir_entry_results| {
                dir_entry_results.iter_mut().for_each(|dir_entry_result| {
                    if let Ok(dir_entry) = dir_entry_result {
                        let metadata = dir_entry.metadata();
                        dir_entry.client_state = Some(metadata);
                    }
                })
            }
        })
        .parallelism(jwalk::Parallelism::RayonExistingPool {
            pool: jwalk::rayon::ThreadPoolBuilder::new()
                .stack_size(128 * 1024)
                .num_threads(num_cpus::get())
                .thread_name(|idx| format!("dua-fs-walk-{idx}"))
                .build()
                .expect("fields we set cannot fail")
                .into(),
            busy_timeout: None,
        })
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let start = time::Instant::now();
    #[allow(non_snake_case)]
    let showInfos = Arc::new(Mutex::new(Vec::new()));
    let args = Args::parse();
    let mut tasks = vec![];
    let root = Path::new(&args.root);
    for root_entry in read_dir(root)? {
        let root_entry = root_entry?;
        if !args.execlude.is_empty() && (args
            .execlude
            .contains(&root_entry.file_name().to_str().unwrap().to_string())
             || args.execlude.contains(&root_entry.path().to_str().unwrap().to_string())
        )
        {
            writeln!(stderr(), "\x1b[41m execlude file or dir:{}\x1b[0m", root_entry.path().display()).expect("failed to write stderr");
            stderr().flush()?;
            continue;
        }
        #[allow(non_snake_case)]
        let showInfos = showInfos.clone();
        let task = tokio::spawn(async move {
            let total_filesize = iter_from_path(&root_entry.path())
                .into_iter()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_type().is_file())
                .map(|entry| {
                    file_real_size_fast(entry.path(), &entry.client_state.unwrap().unwrap())
                        .unwrap_or(0)
                })
                .sum::<u64>();
            writeln!(stdout(),
                     "{:>10},{}",
                     humansize::format_size(total_filesize, humansize::BINARY),
                     root_entry.path().display()
            ).expect("failed to write stdout");
            #[allow(non_snake_case)]
            let mut showInfos = showInfos.lock().await;
            let total_filesize_string = humansize::format_size(total_filesize, humansize::BINARY);
            showInfos.push(ShowInfo {
                total_filesize_string: total_filesize_string.clone(),
                filesSize_type: total_filesize_string.split(" ").last().unwrap().to_string(),
                path: root_entry.path().display().to_string(),
            });
        });
        tasks.push(task);
    }
    futures::future::join_all(tasks).await;

    let mut output_path = PathBuf::from(format!("du-jwalk-{}.csv", chrono::Local::now().format("%Y%m%d")));
    if let Some(pathBuf) = args.output {
        output_path =  pathBuf;
    }
    println!("CSV files write to {:?}",output_path.display());
    let mut wtr = csv::Writer::from_path(output_path)?;
    let mut show_infos = showInfos.lock().await;
    show_infos.sort_by(|a, b| a.filesSize_type.cmp(&b.filesSize_type).then(a.path.cmp(&b.path)));
    wtr.write_record(&["存储路径", "存储类型", "存储大小"])?;
    for show_info in show_infos.iter() {
        wtr.write_record(&[
            &show_info.path,
            &show_info.filesSize_type,
            &show_info.total_filesize_string
        ])?;
    }
    wtr.flush()?;
    drop(show_infos);
    println!("total time:{}", humantime::format_duration(start.elapsed()));
    Ok(())
}
