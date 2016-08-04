extern crate uuid;

use rustc_serialize::{json, Decodable};
use itertools::Itertools;
use crypto::md5::Md5;
use crypto::digest::Digest;

use std::io::prelude::*;
use std::fs::{File, DirBuilder, remove_file};
use std::path::{Path};
use std::collections::hash_map::HashMap;

use types::*;

pub trait FileDownloader {
    fn get_file_list(&mut self, root_folder: &uuid::Uuid) -> Result<Vec<FileResponse>, DriveError>;
    fn resolve_error(&mut self, resp_string: &str) -> Result<(), DriveError>;
    fn verify_checksum(&mut self, fd: &FileData, checksum: &String) -> Result<FileCheckResponse, DriveError>;
    fn create_new_file(&mut self, fd: &FileData, file_path: &Path, metadata_path_str:&Path) -> Result<u64, DriveError>;
}

pub struct FileTree<'a, 'b> {
    pub files: HashMap<uuid::Uuid, u64>,
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

            if let Some(parent) = self.files.get(&root_folder_id) {
                println!("found parent {}, adding new child {:?}", root_folder_id, new_path);
                let child_list = self.child_map.entry(*parent).or_insert(Vec::new());
                child_list.push(inode);
            } else {
                println!("no parent inode in file list");
            }

            let fd =  FileData {
                id: fr.uuid,
                kind: fr.kind,
                path: new_path.to_owned(),
                inode: inode,
                parent_inode: parent_inode,
                size: None,
                source_data: fr.source_data,
            };

            self.files.entry(fr.uuid.clone()).or_insert(inode);
            self.inode_map.entry(inode).or_insert(fd.clone());
            self.child_map.entry(inode).or_insert(Vec::new());
            self.parent_map.entry(inode).or_insert(parent_inode);

            if fd.kind == FileType::Directory {
                println!("getting the next directory's files, {}", new_path.to_string_lossy());
                // create the directory in the system filesystem
                try!(dir_builder.create(&new_path));
                // then recurse to retrieve children files
                try!(self.get_files(&new_path, &fd.id.clone(), fd.inode));
            } else {
            // we're working with a file, not a folder, so we need to save it to the system
                let mut metadata_path = new_path.clone();
                metadata_path.set_file_name(".".to_string() + &fr.name);

                match self.download_and_save_file(&new_path, &metadata_path, &mut fd.clone()) {
                    Ok(_) => {
                        let mut fr_new = self.inode_map.get_mut(&fd.inode).unwrap();
                        match File::open(&metadata_path)
                        {
                            Ok(mut metahandle) => {
                                match read_json_to_type(&mut metahandle) as Result<FileCheckResponse, DriveError> {
                                    Ok(fc) => {
                                        fr_new.size = Some(fc.size.parse::<u64>().unwrap());
                                    },
                                    Err(error) => {
                                        println!("couldn't parse metadata file, {}; err: {:?}", new_path.to_string_lossy(), error);
                                        fr_new.size = Some(0);
                                    }
                                }
                            },
                            Err(error) => {
                                println!("no metadata file for {}; err: {:?}", new_path.to_string_lossy(), error);
                                fr_new.size = Some(0);
                            }
                        };
                    },
                    Err(error) => {
                        println!("error when saving or downloading file: {:?}", error);
                        println!("deleting metadata, and trying a fresh save");
                        try!(remove_file(&metadata_path));
                        try!(self.file_downloader.resolve_error(&error.response.expect("no response in error"))
                            .and(self.download_and_save_file(&new_path, &metadata_path, &mut fd.clone())));
                    }
                };
            }
        }

        Ok(())
    }

    fn download_and_save_file(&mut self, file_path: &Path, metadata_path_str: &Path, fd: &mut FileData) -> Result<(), DriveError> {
        // try to open the metadata file, if it already exists
        let size = match File::open(metadata_path_str.clone()) {
            // the metadata file doesn't yet exist, so the file shouldn't exist either because the
            // two files are created at the same time: create_new_file(), so we'll download it
            Err(_) => {
                try!(self.file_downloader.create_new_file(fd, file_path, metadata_path_str))
            },
            Ok(mut dotf) => {
                // if it does, we'll read the file to see if the checksum matches what we have on
                // the server
                let fc = try!((read_json_to_type(&mut dotf) as Result<FileCheckResponse, DriveError>).or_else(|err| {
                    println!("{:?}", err);
                    let mut file_string = String::new();
                    try!(dotf.read_to_string(&mut file_string));
                    Err(DriveError {
                        kind: DriveErrorType::Tester,
                        response: Some(file_string)
                    })
                }));
                try!(self.file_downloader.verify_checksum(&fd, &fc.md5Checksum).or_else(|_| -> Result<FileCheckResponse, DriveError> {
                    println!("updating metadata for {:?}", file_path);
                    // if the checksum fails, we need to redownload it
                    try!(self.file_downloader.create_new_file(fd, file_path, metadata_path_str));
                    self.file_downloader.verify_checksum(&fd, &fc.md5Checksum)
                }));

                fc.size.parse::<u64>().unwrap()
            }
        };

        fd.size = Some(size);

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

fn read_json_to_type<J: Read, T: Decodable>(json: &mut J) -> Result<T, DriveError> {
    let mut resp_string = String::new();
    try!(json.read_to_string(&mut resp_string));
    json::decode(&resp_string).map_err(From::from)
}

fn convert_pathVec_to_pathString(path_vec: Vec<String>) -> String {
    // convert the path vector to a string for file/folder creation in the system filesystem
    path_vec.iter()
            .intersperse(&"/".to_owned())
            .fold("".to_owned(), |acc, ref filename| acc + &filename.clone())
}

fn convert_pathString_to_pathVec(path_string: String) -> Vec<String> {
    path_string.split('/').map(str::to_string).collect::<Vec<String>>()

}

fn convert_pathVec_to_metaData_pathStr(path: Vec<String>) -> String {
    let mut metadata_new_root_folder = path.clone();
    // remove the filename from the end of the path
    let name = metadata_new_root_folder.pop().expect("there was no file for metadata to belong to");
    // add a dot to the filename and reattach
    // maybe there's a method for this
    metadata_new_root_folder.push(".".to_owned() + &name.clone());
    convert_pathVec_to_pathString(metadata_new_root_folder)
}
