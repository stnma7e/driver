#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(unused_variables)]

extern crate hyper;
extern crate rustc_serialize;
extern crate mime;
extern crate url;
extern crate itertools;
extern crate crypto;
extern crate driver;
extern crate fuse;
extern crate libc;
extern crate time;

use hyper::{Client, client};
use hyper::header::{ContentType, Authorization, Bearer};
use mime::{Mime, TopLevel, SubLevel};

use rustc_serialize::json::{Json, ToJson, Decoder, as_pretty_json};
use rustc_serialize::{json, Decodable};
use itertools::Itertools;
use crypto::md5::Md5;
use crypto::digest::Digest;

use std::io::prelude::*;
use std::io;
use std::fs::{File, DirBuilder, remove_file};
use std::collections::hash_map::HashMap;
use std::os::unix::io::{IntoRawFd};

use std::path::Path;
use libc::{ENOENT, ENOSYS};
use time::Timespec;
use fuse::{FileAttr, FileType, Filesystem, Request, ReplyAttr, ReplyEntry, ReplyDirectory, ReplyData, ReplyOpen};

use driver::types::*;

const CLIENT_ID: &'static str = "";
const CLIENT_SECRET: &'static str = "";
const CACHE_FILE: &'static str = "access";


pub struct FileTree<'a> {
    client: Client,
    auth_data: AuthData<'a>,
    files: HashMap<String, u64>,
    child_map: HashMap<u64, Vec<u64>>,
    inode_map: HashMap<u64, FileResponse>,
    current_inode: u64,
}

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
                        let fileType =
                            if child.mimeType == "application/vnd.google-apps.folder" {
                                FileType::Directory
                            } else {
                                FileType::RegularFile
                            };

                        let size = child.size.unwrap_or(0);

                        if let Some(inode) = child.inode {
                            let ts = Timespec::new(0,0);
                            let attr = FileAttr {
                                ino: child.inode.unwrap(),
                                size: size,
                                blocks: size/512,
                                atime: ts,
                                mtime: ts,
                                ctime: ts,
                                crtime: ts,
                                kind: fileType,
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
                println!("here");
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

fn main() {
    let c = Client::new();

    let tr = || -> io::Result<TokenResponse> {
        let mut handle = try!(File::open(CACHE_FILE)
            .or_else(|_| -> Result<std::fs::File, std::io::Error> {
                let mut handle = try!(File::create(CACHE_FILE));
                let (tr, resp_string) = request_new_access_code(&c);
                println!("{}", resp_string);
                try!(handle.write_all(resp_string.as_bytes()));
                Ok(handle)
            })
        );

        let mut access_string = String::new();
        try!(handle.read_to_string(&mut access_string));
        println!("{}", access_string);
        Ok(json::decode(&access_string)
            .expect("reading the tokenResponse from the access file failed. is this really a TokenResponse?"))
    }().expect("failure in reading access file");

    let mut ft = FileTree {
        files: HashMap::new(),
        client: Client::new(),
        auth_data: AuthData {
            tr: tr.clone(),
            client_id: CLIENT_ID.to_owned(),
            client_secret: CLIENT_SECRET.to_owned(),
            cache_file_path: CACHE_FILE,
        },
        inode_map: HashMap::new(),
        child_map: HashMap::new(),
        current_inode: 1,
    };

//    let root_folder = (vec![], "0B7TtU3YsiIjTTS1oUE5wZFpsYVk");
    let root_folder = (vec!["rot".to_string()], "0B7TtU3YsiIjTWjBOM0YwYkVBa1U");
//    let root_folder = (vec!["rot".to_string()], "0B7TtU3YsiIjTeHJGR1VKMHB3cWs");

    // we're probably dealing with the root folder, so we need to make it's own parent
    ft.files.entry(root_folder.1.to_string()).or_insert(ft.current_inode);
    ft.child_map.entry(ft.current_inode).or_insert(Vec::new());
    println!("{:?}", ft.files);
    ft.current_inode += 1;

    ft.get_files(root_folder).expect("this shit fucked up");

    println!("{:?}", ft.files);
    println!("{:?}", ft.inode_map);

    fuse::mount(ft, &"root.2", &[]);
}

impl<'a> FileTree<'a> {
    fn get_file_list(&mut self, root_folder: &str) -> Result<json::Array, DriveError> {
        let mut resp = try!(self.client.get(&format!("https://www.googleapis.com/drive/v3/files\
                              ?corpus=domain\
                              &pageSize=100\
                              &q=%27{}%27+in+parents\
                              +and+trashed+%3D+false"
                              , root_folder))
                          .header(Authorization(Bearer{token: self.auth_data.tr.access_token.clone()}))
                          .send());

        // convert the response to a string, then to a JSON object
        let mut resp_string = String::new();
        resp.read_to_string(&mut resp_string);
        let fr_data = try!(Json::from_str(&resp_string));
        let fr_obj  = try!(fr_data.as_object()
                                  .ok_or(DriveError::JsonObjectify));

        //println!("{}", resp_string);

        let files = try!(fr_obj.get("files").ok_or(DriveError::JsonInvalidAttribute));
        let files_array = try!(files.as_array().ok_or(DriveError::JsonCannotConvertToArray));
        Ok(files_array.clone())

        // maybe the json returned was really an error response
        // so let's try to reauthenticate
//            None => {
//                println!("error when retreiving file list {}", resp_string);
//                match self.resolve_error(&resp_string) {
                    // if resolve_error did something (could be a re-authorization request, etc.), we can
//                    // try to get the file list again
//                    Ok(_) => self.get_file_list(root_folder),
//                    // otherwise resolve_error encountered some other error, and was not able to reslove
//                    // anything
//                    Err(error) => Err(format!("there was no files attribute in this response, {}; resolution failed with: {}", resp_string, error))
//                }
//            }
//        }
    }

    fn get_files(&mut self, root_folder: (Vec<String>, &str)) -> Result<(), DriveError> {
        let files = try!(self.get_file_list(&root_folder.1.clone()));
        let mut dir_builder = DirBuilder::new();
        dir_builder.recursive(true);

        println!("{:?}", files);

        for i in files.iter() {
            // we'll try to decode each file's metadata JSON object in memory to a FileResponse struct
            let mut decoder = Decoder::new(i.clone());
            let mut fr: FileResponse = try!(Decodable::decode(&mut decoder));

            let mut new_root_folder = root_folder.0.clone();
            // add the file we're working with to the path for the trie
            new_root_folder.push(fr.name.clone());
            let new_path_str = convert_pathVec_to_pathString(new_root_folder.clone());
            let metadata_path_str = convert_pathVec_to_metaData_pathStr(new_root_folder.clone());
            fr.inode = Some(self.current_inode);
            fr.path_string = Some(new_path_str.clone());

            if let Some(parent) = self.files.get(root_folder.1) {
                println!("found parent {}, adding new child {}", root_folder.1, fr.name);
                let child_list = self.child_map.entry(*parent).or_insert(Vec::new());
                child_list.push(self.current_inode);
            } else {
                println!("no parent inode in file list");
            }

            self.files.entry(fr.id.clone()).or_insert(self.current_inode);
            self.inode_map.entry(self.current_inode).or_insert(fr.clone());
            self.child_map.entry(self.current_inode).or_insert(Vec::new());
            self.current_inode += 1;

            if fr.mimeType.clone() == "application/vnd.google-apps.folder" {
                println!("getting the next directory's files, {}", new_path_str.clone());
                // create the directory in the system filesystem
                dir_builder.create(new_path_str);
                // then recurse to retrieve children files
                self.get_files((new_root_folder.clone(), &fr.id.clone()));
            } else {
            // we're working with a file, not a folder, so we need to save it to the system
                match self.download_and_save_file(new_root_folder.clone(), fr.clone()) {
                    Ok(_) => {
                        let mut fr_new = self.inode_map.get_mut(&(self.current_inode-1)).unwrap();
                        match File::open(&metadata_path_str)
                        {
                            Ok(mut metahandle) => {
                                match read_json_to_type(&mut metahandle) as Result<FileCheckResponse, DriveError> {
                                    Ok(fc) => {
                                        fr_new.size = Some(fc.size.parse::<u64>().unwrap());
                                    },
                                    Err(error) => {
                                        println!("couldn't parse metadata file, {}", new_path_str.clone());
                                        fr_new.size = Some(0);
                                    }
                                }
                            },
                            Err(error) => {
                                println!("no metadata file for {}", new_path_str.clone());
                                fr_new.size = Some(0);
                            }
                        };
                    },
                    Err(error) => {
                        println!("error when saving or downloading file: {:?}", error);
                        println!("deleting metadata, and trying a fresh save");
                        remove_file(&metadata_path_str);
                        self.download_and_save_file(new_root_folder, fr);
                    }
                };
            }
        }

        Ok(())
    }

    fn resolve_error(&mut self, resp_string: &String) -> Result<(), String> {
        println!("attempting to resolve error response: {}", resp_string);

        let err_data = match Json::from_str(&resp_string) {
            Ok(fr) => fr,
            Err(error) => panic!("cannot read string as Json; error: {}, invalid response: {}", error, resp_string)
        };
        let err_obj  = match err_data.as_object() {
            Some(fr) => fr,
            None => panic!("JSON data returned was not an objcet")
        };

        match err_obj.get("error") {
            Some(error) => match error.as_object().expect("error attribute not an object").get("errors").expect("no errors array").as_array() {
                Some(errors) => {
                for i in errors {
                    let mut decoder = Decoder::new(i.clone());
                    let err: ErrorDetailsResponse = match Decodable::decode(&mut decoder) {
                        Ok(err) => err,
                        Err(error) => panic!("could not decode errorDetailsResponse, error: {}, attempted edr: {}", error, i)
                    };

                    if err.reason   == "authError"           &&
                       err.message  == "Invalid Credentials" &&
                       err.location == "Authorization" {
                        let resp = self.client.post("https://www.googleapis.com/oauth2/v3/token")
                                              .header(ContentType(Mime(TopLevel::Application, SubLevel::WwwFormUrlEncoded, vec![])))
                                              //.header(Host{hostname: "www.googleapis.com".to_owned(), port: None})
                                              .body(&format!("&client_id={}\
                                                    &client_secret={}\
                                                    &refresh_token={}\
                                                    &grant_type=refresh_token", self.auth_data.client_id, self.auth_data.client_secret, self.auth_data.tr.refresh_token))
                                              .send();
                        let mut resp = match resp {
                            Ok(resp) => resp,
                            Err(error) => panic!("{}", error)
                        };
                        self.auth_data.tr.access_token = {
                            let mut ref_string = String::new();
                            resp.read_to_string(&mut ref_string);
                            let ref_data = match Json::from_str(&ref_string) {
                                Ok(fr) => fr,
                                Err(error) => panic!("cannot read string as Json; invalid response, {}", error)
                            };
                            let ref_obj  = match ref_data.as_object() {
                                Some(fr) => fr,
                                None => panic!("JSON data returned was not an objcet")
                            };
                            if let Some(acc) =  ref_obj.get("access_token") {
                                acc.as_string().unwrap().to_owned()
                            } else {
                                return Err(format!("second error during attempted resolution: {}", error))
                            }
                        };

                        // we'll open our access_cache file to read its current contents
                        let mut f = File::open(self.auth_data.cache_file_path.clone()).unwrap();
                        let mut access_string = String::new();
                        if let Ok(_) = f.read_to_string(&mut access_string) {

                            // if the current access file reads as it should, we'll copy the
                            // current token response and edit the access_token field for later use
                            let mut tr: TokenResponse = match json::decode(&access_string) {
                                Ok(tr) => tr,
                                Err(error) => panic!()
                            };
                            tr.access_token = self.auth_data.tr.access_token.clone();
                            let tr_json = tr.to_json();
                            let tr_str = format!("{}", as_pretty_json(&tr_json));

                            // then we'll truncate the file and paste in the updated token response
                            // preserving all the other unused data
                            let mut f = File::create(self.auth_data.cache_file_path.clone()).unwrap();
                            f.write_all(tr_str.as_bytes());
                        };
                    };
                }
                Ok(())},
                None => panic!("the response given to resolve_error() had no array of errors")
            },
            None => {
                println!("the response given to resolve_error() did not contain an error attribute");
                Err("there was no error attribute for the error sent to resolve_error()".to_owned())
            }
        }
    }

    fn download_and_save_file(&mut self, file_path: Vec<String>, fr: FileResponse) -> Result<(), DriveError> {
        let new_path_str = convert_pathVec_to_pathString(file_path.clone());
        let metadata_path_str = convert_pathVec_to_metaData_pathStr(file_path.clone());

        if fr.mimeType == "application/vnd.google-apps.document"
           || fr.mimeType == "application/vnd.google-apps.form"
           || fr.mimeType == "application/vnd.google-apps.drawing"
           || fr.mimeType == "application/vnd.google-apps.fusiontable"
           || fr.mimeType == "application/vnd.google-apps.map"
           || fr.mimeType == "application/vnd.google-apps.presentation"
           || fr.mimeType == "application/vnd.google-apps.spreadsheet"
           || fr.mimeType == "application/vnd.google-apps.sites" {
            println!("unsupported google filetype");
            return Err(DriveError::UnsupportedDocumentType)
        }

        // try to open the metadata file, if it already exists
        match File::open(metadata_path_str.clone()) {
            Ok(mut dotf) => {
                // if it does, we'll read the file to see if the checksum matches what we have on
                // the server
                let fc = try!((read_json_to_type(&mut dotf) as Result<FileCheckResponse, DriveError>).or_else(|_| {
                    println!("here more {}", metadata_path_str.clone());
                    let mut strg = String::new();
                    dotf.read_to_string(&mut strg);
                    println!("{}", strg);
                    Err(DriveError::Tester)
                }));
                try!(self.verify_file_checksum(&fr, &fc.md5Checksum).or_else(|_| -> Result<String, DriveError> {
                    println!("updating metadata for {}", new_path_str);
                    // if the checksum fails, we need to redownload it
                    try!(self.create_new_file(fr.clone(), file_path.clone()));
                    Ok("".to_string())
                }));
                    // if the file exists, but doesn't read (this should be an error), but we'll
                    // checksum the file (if we have it) on the system, and compare it to the
                    // server, if all is OK, create a new metadata file
    //                Err(error) => {
//                        println!("reading the filecheck from {} failed, error: {:?}", new_path_str.clone(), error);
//                        println!("creating new metadata file");
//                        let mut dotf = try!(File::create(metadata_path_str.clone()));
//                        // if the file exists, get the checksum
//                        let current_file_checksum = try!(get_file_checksum(file_path.clone()));
//                        let equal = try!(self.verify_file_checksum(fr.clone(), &current_file_checksum));
//                        // if the checksum matches that from Drive, then file is downloaded
//                        // correctly, and all we need to do is cache the checksum
//                        if equal {
//                            try!(dotf.write_all(&resp_string.into_bytes()));
//                        // the file's checksum doesn't match the server's, so we need to
//                        // re-download it
//                        } else {
//                            println!("checksum match failed, redownloading file {}", new_path_str);
//                            self.create_new_file(fr, file_path);
//                        }
////                        // otherwise, the file doesn't exist yet, so just create it
////                        } else {
// //                           self.create_new_file(fr, file_path);
//  //                      }
    //                }
    //            };
            },
            // the metadata file doesn't yet exist, so the file shouldn't exist either because the
            // two files are created at the same time: create_new_file(), so we'll download it
            Err(_) => {
                println!("creating new file with metadata: {}", metadata_path_str);
                try!(self.create_new_file(fr, file_path));
            }
        };

        Ok(())
    }

    fn create_new_file(&mut self, fr: FileResponse, file_path: Vec<String>) -> Result<(), DriveError> {
        let new_path_str = convert_pathVec_to_pathString(file_path.clone());
        let metadata_path_str = convert_pathVec_to_metaData_pathStr(file_path.clone());

        let mut dotf = try!(File::create(metadata_path_str.clone()));
        try!(get_file_checksum(file_path.clone()).or_else(|_| {
            println!("here");
            Err(DriveError::Tester)
        }).and_then(|current_file_checksum| {
            self.verify_file_checksum(&fr, &current_file_checksum).and_then(|resp_string| {
                try!(dotf.write_all(&resp_string.into_bytes()));
                // this necessary for the sam reason that the Ok(()) at the bottom of the function is:
                // a mixture of io::Result and another Result type in the try block
                Ok(())
            })
        }).or_else(|_| -> Result<(), DriveError>  {
                    // if the checksum matches that from Drive, then file is downloaded
                    // correctly, and all we need to do is cache the checksum
                    // the file data doesn't yet exist, so we'll download it from scratch

                    println!("downloading new file, creating new metadata file: {}", metadata_path_str);

                    let mut f = try!(File::create(new_path_str));

                    let mut resp = try!(self.client.get(&format!("https://www.googleapis.com/drive/v3/files/{}\
                                ?alt=media", fr.id.clone()))
                                         .header(Authorization(Bearer{token: self.auth_data.tr.access_token.clone()}))
                                         .send());

                    let mut resp_string = Vec::<u8>::new();
                    try!(resp.read_to_end(&mut resp_string));
                    println!("len: {}", resp_string.len());
                    try!(f.write_all(&resp_string.clone()));

                    let md5_result = {
                        let mut md5 = Md5::new();
                        md5.input(&resp_string);
                        md5.result_str()
                    };

                    let resp_string = try!(self.verify_file_checksum(&fr, &md5_result));
                    try!(dotf.write_all(&resp_string.into_bytes()));

                    Ok(())
            })
        );

        // i think this is necessary because get_file_checksum() has type io::Result, so it doesn't fit the type of the
        // rest of the try block
        Ok(())
    }

    fn verify_file_checksum(&mut self, fr: &FileResponse, checksum: &String) -> Result<String, DriveError> {
        let mut resp = self.send_authorized_request((&format!(
                        "https://www.googleapis.com/drive/v3/files/{}\
                        ?fields=md5Checksum%2Csize"
                        , fr.id.clone())));
//                self.resolve_error(&resp_string);
//                match self.send_authorized_request_to_json(
//                    format!("https://www.googleapis.com/drive/v3/files/{}\
//                             ?fields=md5Checksum%2Csize", fr.id.clone())).1 {
//                    Ok(fcr) => fcr,
//                    Err(error) => {
//                        println!("no valid response from Drive, error: {}", error);
//                        return (false, resp_string.clone())
//                    }
//                }

        let mut resp_string = String::new();
        try!(resp.read_to_string(&mut resp_string));
        let fcr: FileCheckResponse = try!(json::decode(&resp_string));

        let same = checksum == &fcr.md5Checksum;
        if !same {
            println!("{} =? {}", checksum, fcr.md5Checksum);
            println!("same?: {:?}", same);
            println!("size: {}", fcr.size);
            return Err(DriveError::JsonObjectify)
        }
        Ok(resp_string)
    }

    fn send_authorized_request_to_json<T: Decodable>(&mut self, url: &str) -> Result<T, DriveError> {
        read_json_to_type(&mut self.send_authorized_request(url))
    }

    fn send_authorized_request(&mut self, url: &str) -> client::Response {
        let maybe_resp = self.client.get(url)
                             .header(Authorization(Bearer{token: self.auth_data.tr.access_token.clone()}))
                             .send();
        match maybe_resp {
            Ok(resp) => resp,
            Err(error) => {
                panic!("error when receiving response during file download, {}", error);
            }
        }
    }
}

fn get_file_checksum(file_path: Vec<String>) -> io::Result<String> {
    let mut f = try!(File::open(convert_pathVec_to_pathString(file_path)));
    let mut f_str = String::new();

    try!(f.read_to_string(&mut f_str));

    let mut md5 = Md5::new();
    md5.input_str(&f_str);
    Ok(md5.result_str())
}

fn read_json_to_type<J: Read, T: Decodable>(json: &mut J) -> Result<T, DriveError> {
        let mut resp_string = String::new();
        json.read_to_string(&mut resp_string);
        match json::decode(&resp_string) {
            Ok(t) => Ok(t),
            Err(err) => Err(DriveError::JsonCannotDecode(err))
        }
}

fn request_new_access_code(c: &Client) -> (TokenResponse, String) {
    // the space after googleusercontent.com is necessary to seperate the link from the rest of the
    // prompt
    println!("Visit https://accounts.google.com/o/oauth2/v2/auth\
                    ?scope=email%20profile%20https://www.googleapis.com/auth/drive\
                    &redirect_uri=urn:ietf:wg:oauth:2.0:oob\
                    &response_type=code\
                    &client_id={} \
             to receive the access code.", CLIENT_ID);
    let mut code_string = String::new();
    match io::stdin().read_line(&mut code_string) {
        Ok(_) => {}
        Err(error) => println!("error when reading access code: {}", error),
    }

    let mut resp = c.post("https://accounts.google.com/o/oauth2/token")
        .header(ContentType(Mime(TopLevel::Application, SubLevel::WwwFormUrlEncoded, vec![])))
        //.header(Host{hostname: "www.googleapis.com".to_owned(), port: None})
        .body(&format!("code={}\
               &client_id={}\
               &client_secret={}\
               &redirect_uri=urn:ietf:wg:oauth:2.0:oob\
               &grant_type=authorization_code"
               , code_string, CLIENT_ID, CLIENT_SECRET))
        .send()
        .unwrap();
    print!("{}\n{}\n{}\n\n", resp.url, resp.status, resp.headers);

    let mut resp_string = String::new();
    resp.read_to_string(&mut resp_string);
    match json::decode(&resp_string) {
        Ok(tr) => return (tr, resp_string),
        Err(error) => {
            println!("response: {}", resp_string);
            panic!("{}", error)
        }
    }
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
