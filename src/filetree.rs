extern crate uuid;

use crypto::md5::Md5;
use crypto::digest::Digest;

use std::io::prelude::*;
use std::fs::{File, DirBuilder};
use std::path::{Path};
use std::collections::hash_map::HashMap;
use uuid::Uuid;

use time;
use fuse::FileAttr;

use types::*;

pub trait FileDownloader {
    fn get_file_list(&mut self, root_folder: &uuid::Uuid) -> Result<Vec<FileResponse>, DriveError>;
    fn resolve_error(&mut self, resp_string: &str) -> Result<(), DriveError>;
    fn verify_checksum(&self, fd: &Uuid, checksum: &String) -> Result<FileCheckResponse, DriveError>;
    fn retreive_file(&mut self, uuid: &Uuid, parent_uuid: &Uuid) -> Result<u64, DriveError>;
    fn create_local_file(&mut self, fd: &FileData, file_path: &Path, metadata_path_str: &Path) -> Result<u64, DriveError>;
    fn read_file(&self, uuid: &Uuid) -> Result<Vec<u8>, DriveError>;
}

pub struct FileTree<'a, 'b> {
    pub child_map: HashMap<u64, Vec<u64>>,
    pub inode_map: HashMap<u64, FileData>,
    pub parent_map: HashMap<u64, u64>,
    pub current_inode: u64,
    pub root_folder: &'a str,

    pub file_downloader: &'b mut FileDownloader,
}

impl<'a, 'b> FileTree<'a, 'b> {
    pub fn get_files(&mut self, root_folder_path: &Path, root_folder_id: &uuid::Uuid, parent_inode: u64) -> Result<(), DriveError> {
        let files = try!(
            self.file_downloader.get_file_list(root_folder_id).or_else(|err| {
                self.file_downloader.resolve_error(&err.response.expect("no response in errorrr"))
                .and(self.file_downloader.get_file_list(root_folder_id))
            })
        );

        let mut dir_builder = DirBuilder::new();
        dir_builder.recursive(true);

        for fr in files {
            let inode = self.current_inode;
            self.current_inode += 1;

            let mut new_path = root_folder_path.to_owned();
            new_path.push(fr.name.clone());
            println!("found parent {}, adding new child {:?}", root_folder_id, new_path);

            let ts = time::now().to_timespec();
            let mut fd = FileData {
                id: fr.uuid,
                path: new_path.to_owned(),
                parent_inode: parent_inode,
                attr: FileAttr {
                    ino: inode,
                    size: 0,
                    blocks: 0,
                    atime: ts,
                    mtime: ts,
                    ctime: ts,
                    crtime: ts,
                    kind: fr.kind.clone(),
                    perm: 0o777,
                    nlink: 0,
                    uid: 1000,
                    gid: 1000,
                    rdev: 0,
                    flags: 0,
                },
                source_data: fr.source_data,
            };

            {
                let parent = try!(self.inode_map.get(&fd.parent_inode).ok_or(DriveError {
                    kind: DriveErrorType::NoSuchInode,
                    response: None,
                }));

                match self.file_downloader.retreive_file(&fd.id, &parent.id) {
                    Ok(size) => {
                        fd.attr.size = size;
                        fd.attr.blocks = size/512;
                    },
                    Err(error) => {
                        println!("error when saving or downloading file: {:?}", error);
                        println!("deleting metadata, and trying a fresh save");
                        try!(self.file_downloader.resolve_error(&error.response.expect("no response in error"))
                            .and(self.file_downloader.retreive_file(&fd.id, &parent.id)));
                    }
                };
            }

            self.inode_map.entry(inode).or_insert(fd.clone());
            self.child_map.entry(inode).or_insert(Vec::new());
            self.parent_map.entry(inode).or_insert(parent_inode);
            self.child_map.entry(parent_inode).or_insert(Vec::new())
                .push(inode);

            if fr.kind == FileType::Directory {
                println!("getting the next directory's files, {}", new_path.to_string_lossy());
                // create the directory in the system filesystem
                try!(dir_builder.create(&new_path));
                // then recurse to retrieve children files
                try!(self.get_files(&new_path, &fd.id.clone(), fd.attr.ino));
            }
        }

        Ok(())
    }
}

pub fn get_file_checksum(file_path: &Path) -> Result<String, DriveError> {
    let mut f = try!(File::open(file_path));
    let mut f_str = Vec::<u8>::new();

    try!(f.read_to_end(&mut f_str));

    let mut md5 = Md5::new();
    md5.input(&f_str);

    Ok(md5.result_str())
}
