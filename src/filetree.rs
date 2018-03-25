extern crate uuid;
extern crate rusqlite;

use crypto::md5::Md5;
use crypto::digest::Digest;

use std::io::prelude::*;
use std::fs::{File};
use std::path::{Path};
use std::collections::hash_map::HashMap;
use uuid::Uuid;

use time;
use fuse::FileAttr;

use types::*;

pub struct FileUpdates {
    pub new_files: Option<Vec<FileResponse>>,
    pub deleted_files: Option<Vec<FileResponse>>
}

pub trait FileDownloader {
    fn get_file_list(&mut self, root_folder: &uuid::Uuid) -> Result<FileUpdates, DriveError>;
    fn resolve_error(&mut self, resp_string: &str) -> Result<(), DriveError>;
    fn verify_checksum(&self, fd: &Uuid, checksum: Option<&str>) -> Result<FileCheckResponse, DriveError>;
    fn retreive_file(&mut self, uuid: &Uuid, parent_uuid: &Uuid) -> Result<u64, DriveError>;
    fn create_local_file(&mut self, parent_uuid: &Uuid, file_path: &Path) -> Result<Uuid, DriveError>;
    fn read_file(&self, uuid: &Uuid) -> Result<Vec<u8>, DriveError>;
    fn write_file(&self, uuid: &Uuid, data: &[u8], offset: u64) -> Result<u32, DriveError>;
    fn flush_file(&self, uuid: &Uuid) -> Result<(), DriveError>;
}

pub struct FileTree<'b> {
    pub child_map: HashMap<u64, Vec<u64>>,
    pub inode_map: HashMap<u64, FileData>,
    pub parent_map: HashMap<u64, u64>,
    pub current_inode: u64,

    pub file_downloader: &'b mut FileDownloader,
    pub conn: rusqlite::Connection,
}

impl<'b> FileTree<'b> {
    fn check_for_new_files(&mut self, parent_folder_id: &uuid::Uuid, parent_inode: u64) -> Result<(), DriveError> {
        println!("\n\n\n");
        let updates = try!(
            self.file_downloader.get_file_list(parent_folder_id)
            .or_else(|err| {
                println!("trying to resolve a getfilelist error");
                self.file_downloader.resolve_error(&err.response.expect(&format!("no response in errorrr: {:?}", err.kind)))
                .and(self.file_downloader.get_file_list(parent_folder_id))
                .or_else(|err| -> Result<FileUpdates, DriveError> {
                    println!("err: {:?}", err);
                    Err(err)
                })
            })
        );

        if let Some(new_files) = updates.new_files {
            for fr in new_files {
                let inode = self.current_inode;
                self.current_inode += 1;

                let kind = if fr.kind == FileType::Directory {
                    "directory"
                } else {
                    "regular"
                };

                let mut size = 0;
                {
                    let parent_ino = self.conn.query_row_named("SELECT ino FROM files WHERE uuid=:uuid"
                        , &[( ":uuid", &fr.parent_uuid.clone().as_bytes().to_vec() )]
                        , |row| -> u64 {
                            row.get::<i32, i64>(0) as u64
                        }
                    ).unwrap();
                    let parent = try!(self.inode_map.get(&parent_ino).ok_or(DriveError {
                        kind: DriveErrorType::NoSuchInode,
                        response: None,
                    }));

                    match self.file_downloader.retreive_file(&fr.uuid, &parent.id) {
                        Ok(s) => {
                            size = s;
                        },
                        Err(error) => {
                            println!("error when saving or downloading file: {:?}", error);
                            println!("deleting metadata, and trying a fresh save");
                            try!(
                                (match error.response {
                                    Some(ref resp) => {
                                        self.file_downloader.resolve_error(resp)
                                    },
                                    None => {
                                        println!("no response in error: {:?}", error.kind);
                                        Err(error)
                                    }
                                })
                                .and(self.file_downloader.retreive_file(&fr.uuid, &parent.id))
                                .or_else(|err| -> Result<u64, DriveError> {
                                    println!("error resolution failed. err2: {:?}", err);
                                    Ok(0)
                                })
                            );
                        }
                    }
                }

                self.conn.execute("INSERT INTO files (ino, uuid, parent_ino, name, size, kind)
                                   VALUES ($1, $2, $3, $4, $5, $6)",
                                   &[ &(inode as i64),
                                      &fr.uuid.clone().as_bytes().to_vec(),
                                      &(parent_inode as i64),
                                      &fr.name.clone(),
                                      &(size as i64),
                                      &kind,
                                    ]
                ).unwrap_or_else(|_| {
//                    println!("file already in filetree db: {}, err: {:?}", fr.name, err);
//                    println!("updating file information");

                    self.conn.execute("UPDATE files
                                       SET name=$1
                                       WHERE uuid=$3"
                        , &[ &fr.name.clone()
                           , &fr.uuid.clone().as_bytes().to_vec()
                           ]
                    ).unwrap_or_else(|err| {
                        println!("couldn't update file in filetree db, err: {:?}", err);
                        0
                    });
                    0
                });


            }
        }

        if let Some(del_files) = updates.deleted_files {
            for fr in del_files {
                self.conn.execute("DELETE FROM files WHERE uuid=:uuid"
                    , &[ &fr.uuid.clone().as_bytes().to_vec() ]
                ).unwrap_or_else(|err| {
                    println!("couldn't delte file: {}, err: {:?}", fr.name, err);
                    0
                });
            }
        }

        Ok(())

    }

    pub fn get_files(&mut self, parent_folder_path: &Path, parent_folder_id: &uuid::Uuid, parent_inode: u64) -> Result<(), DriveError> {
        try!(self.check_for_new_files(parent_folder_id, parent_inode));
        println!("Populating FUSE fs...


        ");
        self._get_files(parent_folder_path, parent_folder_id, parent_inode)
    }

    fn _get_files(&mut self, parent_folder_path: &Path, parent_folder_id: &uuid::Uuid, parent_inode: u64) -> Result<(), DriveError> {
        let files = {
            let mut stmt = try!(self.conn.prepare("SELECT uuid, ino, name, kind, size FROM files
                                                   WHERE parent_ino=:parent_ino"));
            let rows = try!(stmt.query_map_named(&[(":parent_ino", &(parent_inode as i64))]
                , |row| -> FileData {
                    let uuid = Uuid::from_bytes(&row.get::<i32, Vec<u8>>(0)).unwrap();
                    let ino  = row.get::<i32, i64>(1) as u64;
                    let name = row.get::<i32, String>(2);
                    let str_kind = row.get::<i32, String>(3);
                    let size = row.get::<i32, i64>(4) as u64;
                    let ts = time::now().to_timespec();

                    let mut fadsf = FileType::RegularFile;
                    if str_kind == "directory" {
                        fadsf = FileType::Directory;
                    }

                    let mut new_path = parent_folder_path.to_owned();
                    new_path.push(name.clone());

                    FileData {
                        id: uuid,
                        path: new_path,
                        parent_inode: parent_inode,
                        attr: FileAttr {
                            ino: ino,
                            size: size,
                            blocks: size/512,
                            atime: ts,
                            mtime: ts,
                            ctime: ts,
                            crtime: ts,
                            kind: fadsf,
                            perm: 0o777,
                            nlink: 0,
                            uid: 1000,
                            gid: 1000,
                            rdev: 0,
                            flags: 0,
                        },
                        source_data: SourceData::CreatedFile,
                    }
                }
            ));

            let mut files = Vec::new();
            for i in rows {
                files.push(i.unwrap())
            }
            files.clone()
        };

        for fd in files {
            //println!("found parent {}, adding new child {:?}, inode: {}", parent_folder_id, fd.path, fd.attr.ino);

            self.inode_map.entry(fd.attr.ino).or_insert(fd.clone());
            self.child_map.entry(fd.attr.ino).or_insert(Vec::new());
            self.parent_map.entry(fd.attr.ino).or_insert(parent_inode);
            self.child_map.entry(parent_inode).or_insert(Vec::new())
                .push(fd.attr.ino);

            // then recurse to retrieve children files
            if fd.attr.kind == FileType::Directory
            && fd.attr.ino  != 1 {
                try!(self._get_files(&fd.path, &fd.id, fd.attr.ino));
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
