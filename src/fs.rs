extern crate uuid;

use std::path::Path;
use libc::{ENOENT, ENOSYS};
use time;
use time::Timespec;
use fuse::{FileAttr, Filesystem, Request, ReplyAttr, ReplyEntry, ReplyDirectory, ReplyData, ReplyOpen, ReplyEmpty, ReplyWrite, ReplyStatfs, ReplyCreate, ReplyLock, ReplyBmap};
use std::ffi::OsStr;

use std::fs::{File};
use std::os::unix::io::{IntoRawFd, FromRawFd};
use uuid::Uuid;

use filetree::*;
use types::*;

impl<'a, 'b> Filesystem for FileTree<'a, 'b> {
    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        println!("getattr(ino={})", ino);

        if let Some(path) = self.inode_map.get(&ino) {
            let ttl = time::now().to_timespec();
            reply.attr(&ttl, &path.attr);
            return
        } else {
            println!("erro in getattr, ino={}", ino);
            reply.error(ENOSYS);
            return
        }
    }

    fn lookup(&mut self, _req: &Request, parent: u64, name: &Path, reply: ReplyEntry) {
        if let Some(children) = self.child_map.get(&parent) {
            for i in children {
                if let Some(child) = self.inode_map.get(&i) {
                    let path = child.path.clone();
                    if path.file_name().expect(&format!("no file_name {:?}", child)) == name {
                        let ttl = time::now().to_timespec();
                        reply.entry(&ttl, &child.attr, 0);
                        return
                    }
                }
            }
        }

        reply.error(ENOENT)
    }

    fn readdir(&mut self, _req: &Request, ino: u64, fh: u64, offset: u64, mut reply: ReplyDirectory) {
        println!("readdir(ino={}, fh={}, offset={})", ino, fh, offset);
        if offset == 0 {
            reply.add(ino, 0, FileType::Directory, &Path::new("."));
            reply.add(*self.parent_map.get(&ino).expect(&format!("no parent inode for {}", ino)), 1, FileType::Directory, &Path::new(".."));

            let mut new_offest = 1;
            if let Some(children) = self.child_map.get(&ino) {
                new_offest += 1;

                for child_inode in children {
                    if let Some(child) = self.inode_map.get(&child_inode) {
                        reply.add(*child_inode, new_offest, child.attr.kind, &child.path.file_name().expect(&format!("no file_name {:?}", child)));
                    } else {
                        println!("no inode for child {:?}, parent {:?}", child_inode, children);
                        panic!()
                    }
                }

                reply.ok();
                return
            } else {
                println!("fdasd");
            }
        }

        reply.error(ENOENT);
    }

    fn read(&mut self, _req: &Request, ino: u64, fh: u64, offset: u64, size: u32, reply: ReplyData) {
        println!("read(ino={}, fh={}, offset={}, size={})", ino, fh, offset, size);

        if let Some(fd) = self.inode_map.get(&ino) {
            match self.file_downloader.read_file(&fd.id) {
                Ok(data) => {
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

                    println!("{}", d.len());

                    reply.data(&d);
                    return
                },
                Err(err) => {
                    println!("err when reading file, id: {}: {:?}", ino, err)
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
        reply.opened(0, flags)
    }

    fn forget(&mut self, _req: &Request, _ino: u64, _nlookup: u64) {
        println!("forget(ino={})", _ino);
    }
    fn setattr(&mut self, _req: &Request, _ino: u64, _mode: Option<u32>, _uid: Option<u32>, _gid: Option<u32>, _size: Option<u64>, _atime: Option<Timespec>, _mtime: Option<Timespec>, _fh: Option<u64>, _crtime: Option<Timespec>, _chgtime: Option<Timespec>, _bkuptime: Option<Timespec>, _flags: Option<u32>, reply: ReplyAttr) {
        println!("setattr(ino={}, mode={:?}, uid={:?})", _ino, _mode, _uid);
        if let Some(path) = self.inode_map.get(&_ino) {
            let ts = time::now().to_timespec();
            reply.attr(&ts, &path.attr);
            return
        }

        reply.error(ENOENT)
    }
    fn readlink(&mut self, _req: &Request, _ino: u64, reply: ReplyData) { unimplemented!() }
    fn mknod(&mut self, _req: &Request, _parent: u64, _name: &Path, _mode: u32, _rdev: u32, reply: ReplyEntry) {
        println!("mknod(name={:?}, parent={}, mode={})", _name, _parent, _mode);
        reply.error(ENOENT)
    }
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

        unsafe {
            let _: File = FromRawFd::from_raw_fd(_fh as i32);
            // when fd goes out of scope, it will close the file?
        }

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
                    println!("no downloaded folder for root, {}, err: {:?}", self.root_folder, error);
                }
            }
        }

        if let Some(fd) = self.inode_map.get(&ino) {
            match File::open(&fd.path) {
                Ok(handle) => {
                    let h = handle.into_raw_fd();
                    reply.opened(h as u64, _flags);
                    return
                }
                Err(error) => {
                    //println!("no downloaded folder for {}, err: {:?}", fd.path.to_string_lossy(), error);
                }
            }
        } else {
            println!("no inode found in map, {}", ino);
        }

//         reply.error(ENOENT);
        reply.opened(0, _flags)
    }

    fn releasedir(&mut self, _req: &Request, _ino: u64, _fh: u64, _flags: u32, reply: ReplyEmpty) {
        println!("releasedir(ino={}, fh={})", _ino, _fh);

        unsafe {
            let _: File = FromRawFd::from_raw_fd(_fh as i32);
            // when fd goes out of scope, it will close the file?
        }

        reply.ok()
    }
    fn fsyncdir(&mut self, _req: &Request, _ino: u64, _fh: u64, _datasync: bool, reply: ReplyEmpty) { unimplemented!() }
    fn statfs(&mut self, _req: &Request, _ino: u64, reply: ReplyStatfs) {
        println!("statfs(ino={})", _ino);
        reply.error(ENOENT)
    }
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
    fn create(&mut self, _req: &Request, _parent: u64, _name: &Path, _mode: u32, _flags: u32, reply: ReplyCreate) {
        println!("create(name{:?}, parent={}, mode={}, flags={})", _name, _parent, _mode, _flags);

        let inode = self.current_inode;
        self.current_inode += 1;

        let uuid =  Uuid::new_v4();
        let ts = time::now().to_timespec();
        let mut path = {
            let parent = self.inode_map.get(&_parent)
                .expect(&format!("no parent for attempted fs creation of file: parent {}, file {:?}", _parent, _name));
            parent.path.clone()
        };
        path.push(_name);

        let fd = FileData {
            id: uuid,
            path: path,
            parent_inode: _parent,
            attr: FileAttr {
                ino: inode,
                size: 0,
                blocks: 0,
                atime: ts,
                mtime: ts,
                ctime: ts,
                crtime: ts,
                kind: FileType::RegularFile,
                perm: 0o777,
                nlink: 0,
                uid: 1000,
                gid: 1000,
                rdev: 0,
                flags: 0,
            },
            source_data: SourceData::CreatedFile,
        };

        self.child_map.entry(_parent).or_insert(Vec::new())
            .push(inode);
        self.inode_map.entry(inode).or_insert(fd.clone());
        self.child_map.entry(inode).or_insert(Vec::new());
        self.parent_map.entry(inode).or_insert(_parent);

        reply.created(&ts, &fd.attr, fd.attr.ino, 0, 0)
    }
    fn getlk(&mut self, _req: &Request, _ino: u64, _fh: u64, _lock_owner: u64, _start: u64, _end: u64, _typ: u32, _pid: u32, reply: ReplyLock) { unimplemented!() }
    fn setlk(&mut self, _req: &Request, _ino: u64, _fh: u64, _lock_owner: u64, _start: u64, _end: u64, _typ: u32, _pid: u32, _sleep: bool, reply: ReplyEmpty) { unimplemented!() }
    fn bmap(&mut self, _req: &Request, _ino: u64, _blocksize: u32, _idx: u64, reply: ReplyBmap) { unimplemented!() }
}
