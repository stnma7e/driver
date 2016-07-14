use std::path::Path;
use libc::{ENOENT, ENOSYS};
use time::Timespec;
use fuse::{FileAttr, FileType, Filesystem, Request, ReplyAttr, ReplyEntry, ReplyDirectory, ReplyData, ReplyOpen, ReplyEmpty, ReplyWrite, ReplyStatfs, ReplyCreate, ReplyLock, ReplyBmap};
use std::ffi::OsStr;

use std::io::prelude::*;
use std::fs::{File};
use std::os::unix::io::{IntoRawFd};

use filetree::*;

impl<'a> Filesystem for FileTree<'a> {

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        println!("getattr(ino={})", ino);

        let ts = Timespec::new(0, 0);
        let attr = FileAttr {
            ino: 1,
            size: 0,
            blocks: 0,
            atime: ts,
            mtime: ts,
            ctime: ts,
            crtime: ts,
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 0,
            uid: 0,
            gid: 0,
            rdev: 0,
            flags: 0,
        };
        let ttl = Timespec::new(1, 0);
        if ino == 1 {
            reply.attr(&ttl, &attr);
        } else {
            reply.error(ENOSYS);
        }
    }

    fn lookup(&mut self, _req: &Request, parent: u64, name: &Path, reply: ReplyEntry) {
        if let Some(children) = self.child_map.get(&parent) {
            for i in children {
                if let Some(child) = self.inode_map.get(&i) {
                    if child.name.clone() == name.to_str().unwrap() {
                        let file_type =
                            if child.mimeType == "application/vnd.google-apps.folder" {
                                FileType::Directory
                            } else {
                                FileType::RegularFile
                            };

                        let size = child.size.unwrap_or(0);

                        if child.inode.is_some() {
                            let ts = Timespec::new(0,0);
                            let attr = FileAttr {
                                ino: child.inode.unwrap(),
                                size: size,
                                blocks: size/4096,
                                atime: ts,
                                mtime: ts,
                                ctime: ts,
                                crtime: ts,
                                kind: file_type,
                                perm: 0o755,
                                nlink: 0,
                                uid: 1000,
                                gid: 1000,
                                rdev: 0,
                                flags: 0,
                            };

                            let ttl = Timespec::new(1, 0);
                            reply.entry(&ttl, &attr, 0);
                            return
                        } else {
                            println!("no inode found for {:?}", i);
                            reply.error(ENOENT);
                            return
                        }
                    }
                }
            }
        }
    }

    fn readdir(&mut self, _req: &Request, ino: u64, fh: u64, offset: u64, mut reply: ReplyDirectory) {
        println!("readdir(ino={}, fh={}, offset={})", ino, fh, offset);
        if offset == 0 {
            reply.add(1, 4, FileType::Directory, &Path::new("."));
            reply.add(2, 5, FileType::Directory, &Path::new(".."));

            if let Some(children) = self.child_map.get(&ino) {
                for child_inode in children {
                    if let Some(child) = self.inode_map.get(&child_inode) {
                        let file_type =
                            if child.mimeType == "application/vnd.google-apps.folder" {
                                FileType::Directory
                            } else {
                                FileType::RegularFile
                            };
                        reply.add(*child_inode, *child_inode, file_type, &Path::new(&child.name));
                    } else {
                        println!("no inode for child {:?}, parent {:?}", child_inode, children);
                        panic!()
                    }
                }

                reply.ok()
            } else {
                reply.error(ENOENT);
                return
            }
        }
    }

    fn read(&mut self, _req: &Request, ino: u64, fh: u64, offset: u64, size: u32, reply: ReplyData) {
        println!("read(ino={}, fh={}, offset={}, size={})", ino, fh, offset, size);

        if let Some(fr) = self.inode_map.get(&ino) {
            match File::open(&fr.path_string.clone().unwrap()) {
                Ok(mut handle) => {
                    let mut data = Vec::<u8>::new();
                    match handle.read_to_end(&mut data) {
                        Ok(_) => (),
                        Err(error) => println!("couldnt read file handle, {}: error, {}", fr.path_string.clone().unwrap(), error),
                    };

                    let offset = if offset >= data.len() as u64 {
                        println!("offset change: {} -> {}", offset, data.len());
                        (data.len() - 1) as u64
                    } else {
                        offset
                    };

                    let real_size: usize = if size as u64 + offset >= data.len() as u64 {
                        (data.len() - offset as usize) as usize
                    } else {
                        size as usize
                    };
                    let d  = data[offset as usize..(offset + real_size as u64) as usize].to_vec();

                    reply.data(&d);
                    return
                },
                Err(_) => {
                    println!("no downloaded file for {}", fr.path_string.clone().unwrap());
                }
            }
        } else {
            println!("no inode found in map, {}", ino);
        }

        reply.error(ENOENT);
    }

    // implement open flags with file handle later
    fn open(&mut self, _req: &Request, ino: u64, flags: u32, reply: ReplyOpen) {
        println!("open(ino={})", ino);

        if let Some(fr) = self.inode_map.get(&ino) {
            match File::open(&fr.path_string.clone().unwrap()) {
                Ok(handle) => {
                    let h = handle.into_raw_fd();
                    reply.opened(h as u64, flags);
                    return
                }
                Err(error) => {
                    println!("no downloaded file for {}", fr.path_string.clone().unwrap());
                }
            }
        } else {
            println!("no inode found in map, {}", ino);
        }

        reply.error(ENOENT);
    }

    fn forget(&mut self, _req: &Request, _ino: u64, _nlookup: u64) {
        println!("forget(ino={})", _ino);
    }
    fn setattr(&mut self, _req: &Request, _ino: u64, _mode: Option<u32>, _uid: Option<u32>, _gid: Option<u32>, _size: Option<u64>, _atime: Option<Timespec>, _mtime: Option<Timespec>, _fh: Option<u64>, _crtime: Option<Timespec>, _chgtime: Option<Timespec>, _bkuptime: Option<Timespec>, _flags: Option<u32>, reply: ReplyAttr) { unimplemented!() }
    fn readlink(&mut self, _req: &Request, _ino: u64, reply: ReplyData) { unimplemented!() }
    fn mknod(&mut self, _req: &Request, _parent: u64, _name: &Path, _mode: u32, _rdev: u32, reply: ReplyEntry) { unimplemented!() }
    fn mkdir(&mut self, _req: &Request, _parent: u64, _name: &Path, _mode: u32, reply: ReplyEntry) { unimplemented!() }
    fn unlink(&mut self, _req: &Request, _parent: u64, _name: &Path, reply: ReplyEmpty) { unimplemented!() }
    fn rmdir(&mut self, _req: &Request, _parent: u64, _name: &Path, reply: ReplyEmpty) { unimplemented!() }
    fn symlink(&mut self, _req: &Request, _parent: u64, _name: &Path, _link: &Path, reply: ReplyEntry) { unimplemented!() }
    fn rename(&mut self, _req: &Request, _parent: u64, _name: &Path, _newparent: u64, _newname: &Path, reply: ReplyEmpty) { unimplemented!() }
    fn link(&mut self, _req: &Request, _ino: u64, _newparent: u64, _newname: &Path, reply: ReplyEntry) { unimplemented!() }
    fn write(&mut self, _req: &Request, _ino: u64, _fh: u64, _offset: u64, _data: &[u8], _flags: u32, reply: ReplyWrite) { unimplemented!() }
    fn flush(&mut self, _req: &Request, _ino: u64, _fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
        println!("flush(ino={}, fh={})", _ino, _fh);
        reply.ok();
    }
    fn release(&mut self, _req: &Request, _ino: u64, _fh: u64, _flags: u32, _lock_owner: u64, _flush: bool, reply: ReplyEmpty) {
        println!("release(ino={}, fh={}, flush={:?})", _ino, _fh, _flush);
        reply.ok();
    }
    fn fsync(&mut self, _req: &Request, _ino: u64, _fh: u64, _datasync: bool, reply: ReplyEmpty) { unimplemented!() }
    fn opendir(&mut self, _req: &Request, ino: u64, _flags: u32, reply: ReplyOpen) {
        println!("opendir(ino={})", ino);

        if ino == 1 {
            match File::open(self.root_folder) {
                Ok(handle) => {
                    let h = handle.into_raw_fd();
                    reply.opened(h as u64, _flags);
                    return
                }
                Err(error) => {
                    println!("no downloaded file for root, {}", self.root_folder);
                }
            }
        }

        if let Some(fr) = self.inode_map.get(&ino) {
            match File::open(&fr.path_string.clone().unwrap()) {
                Ok(handle) => {
                    let h = handle.into_raw_fd();
                    reply.opened(h as u64, _flags);
                    return
                }
                Err(error) => {
                    println!("no downloaded file for {}", fr.path_string.clone().unwrap());
                }
            }
        } else {
            println!("no inode found in map, {}", ino);
        }

        reply.error(ENOENT);
    }

    fn releasedir(&mut self, _req: &Request, _ino: u64, _fh: u64, _flags: u32, reply: ReplyEmpty) {
        println!("releasedir(ino={}, fh={})", _ino, _fh);
        reply.ok()
    }
    fn fsyncdir(&mut self, _req: &Request, _ino: u64, _fh: u64, _datasync: bool, reply: ReplyEmpty) { unimplemented!() }
    fn statfs(&mut self, _req: &Request, _ino: u64, reply: ReplyStatfs) { unimplemented!() }
    fn setxattr(&mut self, _req: &Request, _ino: u64, _name: &OsStr, _value: &[u8], _flags: u32, _position: u32, reply: ReplyEmpty) { unimplemented!() }
    fn getxattr(&mut self, _req: &Request, _ino: u64, _name: &OsStr, reply: ReplyData) {
        println!("getxattr(ino={}, name={:?})", _ino, _name);
    }
    fn listxattr(&mut self, _req: &Request, _ino: u64, reply: ReplyEmpty) { unimplemented!() }
    fn removexattr(&mut self, _req: &Request, _ino: u64, _name: &OsStr, reply: ReplyEmpty) { unimplemented!() }
    fn access(&mut self, _req: &Request, _ino: u64, _mask: u32, reply: ReplyEmpty) {
        println!("access(ino={})", _ino);
        if self.inode_map.contains_key(&_ino) {
            reply.ok()
        } else {
            reply.error(ENOENT)
        }
    }
    fn create(&mut self, _req: &Request, _parent: u64, _name: &Path, _mode: u32, _flags: u32, reply: ReplyCreate) { unimplemented!() }
    fn getlk(&mut self, _req: &Request, _ino: u64, _fh: u64, _lock_owner: u64, _start: u64, _end: u64, _typ: u32, _pid: u32, reply: ReplyLock) { unimplemented!() }
    fn setlk(&mut self, _req: &Request, _ino: u64, _fh: u64, _lock_owner: u64, _start: u64, _end: u64, _typ: u32, _pid: u32, _sleep: bool, reply: ReplyEmpty) { unimplemented!() }
    fn bmap(&mut self, _req: &Request, _ino: u64, _blocksize: u32, _idx: u64, reply: ReplyBmap) { unimplemented!() }
}
