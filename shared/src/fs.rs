use std::{collections::VecDeque, path::Path};

use anyhow::Result;
use tokio::{fs, io, select, sync::{mpsc, oneshot}, time};

pub async fn calculate_size(path: &Path) -> u64 {
	let mut total = 0;
	let mut stack = VecDeque::from([path.to_path_buf()]);
	while let Some(path) = stack.pop_front() {
		let meta = if let Ok(meta) = fs::symlink_metadata(&path).await {
			meta
		} else {
			continue;
		};

		if !meta.is_dir() {
			total += meta.len();
			continue;
		}

		let mut it = if let Ok(it) = fs::read_dir(path).await {
			it
		} else {
			continue;
		};

		while let Ok(Some(entry)) = it.next_entry().await {
			let meta = if let Ok(m) = entry.metadata().await {
				m
			} else {
				continue;
			};

			if meta.is_dir() {
				stack.push_back(entry.path());
			} else {
				total += meta.len();
			}
		}
	}
	total
}

pub fn copy_with_progress(from: &Path, to: &Path) -> mpsc::Receiver<Result<u64, io::Error>> {
	let (tx, rx) = mpsc::channel(1);
	let (tick_tx, mut tick_rx) = oneshot::channel();

	tokio::spawn({
		let (from, to) = (from.to_path_buf(), to.to_path_buf());

		async move {
			let _ = match fs::copy(from, to).await {
				Ok(len) => tick_tx.send(Ok(len)),
				Err(e) => tick_tx.send(Err(e)),
			};
		}
	});

	tokio::spawn({
		let tx = tx.clone();
		let to = to.to_path_buf();

		async move {
			let mut last = 0;
			let mut exit = None;
			loop {
				select! {
					res = &mut tick_rx => exit = Some(res.unwrap()),
					_ = tx.closed() => break,
					_ = time::sleep(time::Duration::from_secs(1)) => (),
				}

				match exit {
					Some(Ok(len)) => {
						if len > last {
							tx.send(Ok(len - last)).await.ok();
						}
						tx.send(Ok(0)).await.ok();
						break;
					}
					Some(Err(e)) => {
						tx.send(Err(e)).await.ok();
						break;
					}
					None => {}
				}

				let len = fs::symlink_metadata(&to).await.map(|m| m.len()).unwrap_or(0);
				if len > last {
					tx.send(Ok(len - last)).await.ok();
					last = len;
				}
			}
		}
	});

	rx
}

// Convert a file mode to a string representation
#[allow(clippy::collapsible_else_if)]
pub fn file_mode(mode: u32) -> String {
	use libc::{S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK, S_IFMT, S_IFSOCK, S_IRGRP, S_IROTH, S_IRUSR, S_ISGID, S_ISUID, S_ISVTX, S_IWGRP, S_IWOTH, S_IWUSR, S_IXGRP, S_IXOTH, S_IXUSR};

	#[cfg(target_os = "macos")]
	let m = mode as u16;
	#[cfg(target_os = "linux")]
	let m = mode;

	let mut s = String::with_capacity(10);

	// File type
	s.push(match m & S_IFMT {
		S_IFBLK => 'b',
		S_IFCHR => 'c',
		S_IFDIR => 'd',
		S_IFIFO => 'p',
		S_IFLNK => 'l',
		S_IFSOCK => 's',
		_ => '-',
	});

	// Owner
	s.push(if m & S_IRUSR != 0 { 'r' } else { '-' });
	s.push(if m & S_IWUSR != 0 { 'w' } else { '-' });
	s.push(if m & S_IXUSR != 0 {
		if m & S_ISUID != 0 { 's' } else { 'x' }
	} else {
		if m & S_ISUID != 0 { 'S' } else { '-' }
	});

	// Group
	s.push(if m & S_IRGRP != 0 { 'r' } else { '-' });
	s.push(if m & S_IWGRP != 0 { 'w' } else { '-' });
	s.push(if m & S_IXGRP != 0 {
		if m & S_ISGID != 0 { 's' } else { 'x' }
	} else {
		if m & S_ISGID != 0 { 'S' } else { '-' }
	});

	// Other
	s.push(if m & S_IROTH != 0 { 'r' } else { '-' });
	s.push(if m & S_IWOTH != 0 { 'w' } else { '-' });
	s.push(if m & S_IXOTH != 0 {
		if m & S_ISVTX != 0 { 't' } else { 'x' }
	} else {
		if m & S_ISVTX != 0 { 'T' } else { '-' }
	});

	s
}
