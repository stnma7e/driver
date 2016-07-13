use std::path::Path;
use libc::{ENOENT, ENOSYS};
use time::Timespec;
use fuse::{FileAttr, FileType, Filesystem, Request, ReplyAttr, ReplyEntry, ReplyDirectory, ReplyData, ReplyOpen};

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
                                blocks: size/512,
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
                        let fileType =
                            if child.mimeType == "application/vnd.google-apps.folder" {
                                FileType::Directory
                            } else {
                                FileType::RegularFile
                            };
                        reply.add(*child_inode, *child_inode, fileType, &Path::new(&child.name));
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

                    let d: Vec<u8> = data[offset as usize..]
                                    .to_vec()
                                    .into_iter()
                                    .take(size as usize)
                                    .collect();
                    reply.data(&d);
                    return
                },
                Err(error) => {
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
}
