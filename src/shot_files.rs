//! F12: where a picture of the app's popup or window goes, and its note (the same rules as
//! shot_save.py of the Omarchy plugins and screenshot.dart of Maprisma Playground).
//!
//! The picture goes to ~/Pictures/<App name>/<app-slug>-YYYYMMDD-HHMMSS.png (the pictures folder
//! from user-dirs.dirs when it is set there). The folder is made 0700 when missing and used only
//! when it is a real folder (not a link) of this user that group and others cannot write to,
//! checked on the opened handle; files are made relative to that handle, never through a link
//! and never over an existing file (-2, -3 … are added, also when only the .txt note has the
//! name), 0600, written completely, fsync'd, and removed again when writing fails. A note is at
//! most 4000 characters; no text, no file. Nothing is sent anywhere and no program is started.
#![allow(dead_code)] // shared by several apps; each uses a part

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

/// Characters of a note at most: a few sentences, not a document.
pub const NOTE_LIMIT: usize = 4000;
/// Bytes of user-dirs.dirs read at most.
pub const USER_DIRS_LIMIT: usize = 64 * 1024;

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a path with a NUL byte"))
}

fn c_name(name: &str) -> io::Result<CString> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a plain file name"));
    }
    CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a name with a NUL byte"))
}

fn fstat(fd: i32) -> io::Result<libc::stat> {
    let mut info: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut info) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

/// A regular file (not a link, FIFO or device), at most `limit` bytes; else None.
pub fn read_regular_nofollow(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let name = c_path(path).ok()?;
    let fd = unsafe {
        libc::open(name.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
    };
    if fd < 0 {
        return None;
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    if fstat(fd.as_raw_fd()).ok()?.st_mode & libc::S_IFMT != libc::S_IFREG {
        return None;
    }
    let mut data = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = unsafe { libc::read(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
        if n < 0 {
            return None;
        }
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..n as usize]);
        if data.len() > limit {
            return None;
        }
    }
    Some(data)
}

/// XDG_PICTURES_DIR from `config`/user-dirs.dirs ("$HOME/…" or an absolute path; "$HOME" alone
/// is no pictures folder of its own), else ~/Pictures.
pub fn pictures_dir_in(home: &Path, config: &Path) -> PathBuf {
    let mut found = home.join("Pictures");
    let Some(data) = read_regular_nofollow(&config.join("user-dirs.dirs"), USER_DIRS_LIMIT) else {
        return found;
    };
    for line in String::from_utf8_lossy(&data).lines() {
        let Some(rest) = line.trim().strip_prefix("XDG_PICTURES_DIR") else { continue };
        let Some(value) = rest.trim_start().strip_prefix('=') else { continue };
        let value = value.trim();
        let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else { continue };
        if value.contains('"') {
            continue;
        }
        let path = if value == "$HOME" || value == "$HOME/" {
            continue;
        } else if let Some(below) = value.strip_prefix("$HOME/") {
            home.join(below)
        } else {
            PathBuf::from(value)
        };
        if path.is_absolute() && path != home && path.parent().is_some() {
            found = path;
        }
    }
    found
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| "/".into())
}

/// The pictures folder of this session.
pub fn pictures_dir() -> PathBuf {
    let home = home();
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    pictures_dir_in(&home, &config)
}

/// "Maprisma Home" -> "maprisma-home".
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(40).collect();
    if out.is_empty() { "screenshot".into() } else { out }
}

/// `YYYYMMDD-HHMMSS` of the local time `seconds` (since 1970).
pub fn stamp_at(seconds: i64) -> String {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t = seconds as libc::time_t;
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return "00000000-000000".into();
    }
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

pub fn stamp_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    stamp_at(now)
}

/// Writes all of `data` through `write`, going on after short writes; a write of 0 bytes fails.
pub fn write_all_with(mut write: impl FnMut(&[u8]) -> io::Result<usize>, data: &[u8]) -> io::Result<()> {
    let mut rest = data;
    while !rest.is_empty() {
        match write(rest) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "the file could not be written")),
            Ok(n) => rest = &rest[n.min(rest.len())..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// A handle on a folder of this user's that only they may write to.
pub struct PrivateFolder {
    fd: OwnedFd,
}

impl PrivateFolder {
    /// Made 0700 when missing (its parents as usual); used only when it is a real folder (opened
    /// with O_DIRECTORY|O_NOFOLLOW, so no link) owned by this user and not writable by group or
    /// others, checked on the opened handle.
    pub fn open(path: &Path) -> io::Result<Self> {
        let refused =
            || io::Error::new(io::ErrorKind::PermissionDenied, format!("{} is not a private folder of yours", shown(path)));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let name = c_path(path)?;
        let fd = unsafe {
            libc::open(name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        };
        if fd < 0 {
            return Err(refused());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let info = fstat(fd.as_raw_fd())?;
        if info.st_mode & libc::S_IFMT != libc::S_IFDIR
            || info.st_uid != unsafe { libc::getuid() }
            || info.st_mode & 0o022 != 0
        {
            return Err(refused());
        }
        Ok(Self { fd })
    }

    fn stat(&self, name: &str) -> io::Result<libc::stat> {
        let name = c_name(name)?;
        let mut info: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatat(self.fd.as_raw_fd(), name.as_ptr(), &mut info, libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info)
    }

    /// Something of that name is in the folder (also a link that points nowhere).
    pub fn taken(&self, name: &str) -> bool {
        !matches!(self.stat(name), Err(error) if error.kind() == io::ErrorKind::NotFound)
    }

    /// A new file of our own (0600, never through a link, never over an existing file), written
    /// completely and synced, or removed again.
    pub fn write_new(&self, name: &str, data: &[u8]) -> io::Result<()> {
        self.write_new_with(name, data, |fd, chunk| {
            let n = unsafe { libc::write(fd, chunk.as_ptr().cast(), chunk.len()) };
            if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
        })
    }

    /// `write_new` with its own write call (tests: short writes, a full disk).
    pub fn write_new_with(
        &self,
        name: &str,
        data: &[u8],
        mut write: impl FnMut(i32, &[u8]) -> io::Result<usize>,
    ) -> io::Result<()> {
        let c = c_name(name)?;
        let fd = unsafe {
            libc::openat(
                self.fd.as_raw_fd(),
                c.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600 as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let out = unsafe { OwnedFd::from_raw_fd(fd) };
        let result = write_all_with(|chunk| write(out.as_raw_fd(), chunk), data).and_then(|()| {
            if unsafe { libc::fsync(out.as_raw_fd()) } != 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
        });
        drop(out);
        if result.is_err() {
            unsafe { libc::unlinkat(self.fd.as_raw_fd(), c.as_ptr(), 0) };
        }
        result
    }
}

/// `<prefix>-<stamp>.png`, or -2, -3 … when it or its note already exists in the folder.
pub fn free_name(folder: &PrivateFolder, prefix: &str, stamp: &str) -> String {
    let mut name = format!("{prefix}-{stamp}.png");
    let mut n = 2;
    while folder.taken(&name) || folder.taken(&format!("{}.txt", &name[..name.len() - 4])) {
        name = format!("{prefix}-{stamp}-{n}.png");
        n += 1;
    }
    name
}

/// The PNG into a new file in the private folder `dir`; returns its path.
pub fn save_picture(dir: &Path, prefix: &str, stamp: &str, png: &[u8]) -> io::Result<PathBuf> {
    let folder = PrivateFolder::open(dir)?;
    let name = free_name(&folder, prefix, stamp);
    folder.write_new(&name, png)?;
    Ok(dir.join(name))
}

/// The note next to the picture (same name, .txt), at most NOTE_LIMIT characters; no text, no
/// file. The folder is checked again and the picture must still be a file there.
pub fn save_note(picture: &Path, text: &str) -> io::Result<Option<PathBuf>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let note: String = text.chars().take(NOTE_LIMIT).collect();
    let (Some(dir), Some(name)) = (picture.parent(), picture.file_name().and_then(|n| n.to_str())) else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a picture"));
    };
    let Some(stem) = name.strip_suffix(".png") else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a picture"));
    };
    let folder = PrivateFolder::open(dir)?;
    match folder.stat(name) {
        Ok(info) if info.st_mode & libc::S_IFMT == libc::S_IFREG => {}
        _ => return Err(io::Error::new(io::ErrorKind::NotFound, "the picture is gone")),
    }
    let txt = format!("{stem}.txt");
    folder.write_new(&txt, format!("{note}\n").as_bytes())?;
    Ok(Some(dir.join(txt)))
}

/// The path with ~ for the home folder, as the note says it.
pub fn shown(path: &Path) -> String {
    let home = home();
    match path.strip_prefix(&home) {
        Ok(rest) if home != Path::new("/") => {
            if rest.as_os_str().is_empty() { "~".into() } else { format!("~/{}", rest.display()) }
        }
        _ => path.display().to_string(),
    }
}

/// RGBA pixels (as iced's window::screenshot gives them) as a PNG.
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> io::Result<Vec<u8>> {
    if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the picture is empty"));
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(rgba).map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)?;
    Ok(out)
}

/// The window's pixels into ~/Pictures/<app name>/<slug>-<stamp>.png.
pub fn save_screenshot(app_name: &str, rgba: &[u8], width: u32, height: u32) -> io::Result<PathBuf> {
    let png = encode_png(rgba, width, height)?;
    save_picture(&pictures_dir().join(app_name), &slug(app_name), &stamp_now(), &png)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shot-test-{}-{name}-{}", std::process::id(), stamp_now()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn pictures_dir_from_user_dirs() {
        let home = temp("home");
        let config = home.join(".config");
        std::fs::create_dir_all(&config).unwrap();
        assert_eq!(pictures_dir_in(&home, &config), home.join("Pictures"));
        std::fs::write(config.join("user-dirs.dirs"), "# x\nXDG_PICTURES_DIR=\"$HOME/Bilder\"\n").unwrap();
        assert_eq!(pictures_dir_in(&home, &config), home.join("Bilder"));
        std::fs::write(config.join("user-dirs.dirs"), "XDG_PICTURES_DIR=\"/srv/pics\"\n").unwrap();
        assert_eq!(pictures_dir_in(&home, &config), PathBuf::from("/srv/pics"));
        for bad in ["XDG_PICTURES_DIR=\"$HOME\"\n", "XDG_PICTURES_DIR=\"$HOME/\"\n", "XDG_PICTURES_DIR=\"rel/pics\"\n"] {
            std::fs::write(config.join("user-dirs.dirs"), bad).unwrap();
            assert_eq!(pictures_dir_in(&home, &config), home.join("Pictures"), "{bad}");
        }
        // Too big, or a link: not read.
        std::fs::write(config.join("user-dirs.dirs"), format!("XDG_PICTURES_DIR=\"/srv/pics\"\n{}", "#".repeat(USER_DIRS_LIMIT))).unwrap();
        assert_eq!(pictures_dir_in(&home, &config), home.join("Pictures"));
        let real = home.join("real.dirs");
        std::fs::write(&real, "XDG_PICTURES_DIR=\"/srv/pics\"\n").unwrap();
        std::fs::remove_file(config.join("user-dirs.dirs")).unwrap();
        std::os::unix::fs::symlink(&real, config.join("user-dirs.dirs")).unwrap();
        assert_eq!(pictures_dir_in(&home, &config), home.join("Pictures"));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn folder_is_private() {
        let base = temp("folder");
        let dir = base.join("Pictures/Maprisma Test");
        let folder = PrivateFolder::open(&dir).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        drop(folder);
        // Group-writable: refused.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(PrivateFolder::open(&dir).is_err());
        // A link to a folder: refused.
        let real = base.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = base.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(PrivateFolder::open(&link).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn names_never_overwrite() {
        let base = temp("names");
        let dir = base.join("pics");
        let first = save_picture(&dir, "maprisma-test", "20261110-120000", b"one").unwrap();
        assert_eq!(first.file_name().unwrap(), "maprisma-test-20261110-120000.png");
        assert_eq!(std::fs::metadata(&first).unwrap().permissions().mode() & 0o777, 0o600);
        let second = save_picture(&dir, "maprisma-test", "20261110-120000", b"two").unwrap();
        assert_eq!(second.file_name().unwrap(), "maprisma-test-20261110-120000-2.png");
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        // A note alone, or a dangling link, also takes the name.
        std::fs::write(dir.join("maprisma-test-20261110-120001.txt"), "x").unwrap();
        std::os::unix::fs::symlink(base.join("nowhere"), dir.join("maprisma-test-20261110-120002.png")).unwrap();
        let folder = PrivateFolder::open(&dir).unwrap();
        assert_eq!(free_name(&folder, "maprisma-test", "20261110-120001"), "maprisma-test-20261110-120001-2.png");
        assert_eq!(free_name(&folder, "maprisma-test", "20261110-120002"), "maprisma-test-20261110-120002-2.png");
        assert!(folder.write_new("maprisma-test-20261110-120002.png", b"x").is_err());
        assert!(!base.join("nowhere").exists());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn short_and_failed_writes() {
        let base = temp("writes");
        let folder = PrivateFolder::open(&base.join("pics")).unwrap();
        // One byte at a time: all of it arrives.
        folder
            .write_new_with("short.png", b"abcdef", |fd, chunk| {
                let n = unsafe { libc::write(fd, chunk.as_ptr().cast(), 1) };
                Ok(n as usize)
            })
            .unwrap();
        assert_eq!(std::fs::read(base.join("pics/short.png")).unwrap(), b"abcdef");
        // A write of 0 bytes, or an error: nothing is left behind.
        assert!(folder.write_new_with("zero.png", b"abc", |_, _| Ok(0)).is_err());
        assert!(!folder.taken("zero.png"));
        let mut calls = 0;
        let full = folder.write_new_with("full.png", b"abc", |fd, chunk| {
            calls += 1;
            if calls > 1 {
                return Err(io::Error::from_raw_os_error(libc::ENOSPC));
            }
            Ok(unsafe { libc::write(fd, chunk.as_ptr().cast(), 1) } as usize)
        });
        assert!(full.is_err());
        assert!(!folder.taken("full.png"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn notes() {
        let base = temp("notes");
        let dir = base.join("pics");
        let picture = save_picture(&dir, "maprisma-test", "20261110-120000", b"png").unwrap();
        assert_eq!(save_note(&picture, "  \n ").unwrap(), None);
        assert!(!dir.join("maprisma-test-20261110-120000.txt").exists());
        let long = "ä".repeat(NOTE_LIMIT + 50);
        let note = save_note(&picture, &format!("  {long} ")).unwrap().unwrap();
        let text = std::fs::read_to_string(&note).unwrap();
        assert_eq!(text.chars().count(), NOTE_LIMIT + 1);
        assert!(text.ends_with('\n'));
        assert_eq!(std::fs::metadata(&note).unwrap().permissions().mode() & 0o777, 0o600);
        // Never over an existing note; never for a picture that is gone.
        assert!(save_note(&picture, "again").is_err());
        assert_eq!(std::fs::read_to_string(&note).unwrap(), text);
        assert!(save_note(&dir.join("gone.png"), "x").is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn png_and_names() {
        let png = encode_png(&[255, 0, 0, 255, 0, 255, 0, 128], 2, 1).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(encode_png(&[0; 7], 2, 1).is_err());
        assert!(encode_png(&[], 0, 0).is_err());
        assert_eq!(slug("Maprisma Video Translator"), "maprisma-video-translator");
        assert_eq!(slug("  ?? "), "screenshot");
        assert_eq!(stamp_at(0).len(), 15);
    }
}
