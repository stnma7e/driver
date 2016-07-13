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

use types::*;

pub const CLIENT_ID: &'static str = "";
pub const CLIENT_SECRET: &'static str = "";

pub struct FileTree<'a> {
    pub client: Client,
    pub auth_data: AuthData<'a>,
    pub files: HashMap<String, u64>,
    pub child_map: HashMap<u64, Vec<u64>>,
    pub inode_map: HashMap<u64, FileResponse>,
    pub current_inode: u64,
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
        try!(resp.read_to_string(&mut resp_string));
        let fr_obj = try!((Json::from_str(&resp_string)).map_err(From::from)
            .and_then(|fr_data: Json| -> Result<json::Object, DriveError> {
                fr_data.into_object()
                .ok_or(DriveError {
                    kind: DriveErrorType::JsonObjectify,
                    response: Some(resp_string.clone())
                })
            }).or_else(|err| {
                println!("there's probably something wrong with the get_file_list response, err: {:?}", err);
                Err(err)
            })
        );

        //println!("{}", resp_string);

        let files = try!(fr_obj.get("files")
                               .ok_or(DriveError {
                                    kind: DriveErrorType::JsonInvalidAttribute,
                                    response: Some(resp_string.clone())
                                }));
        let files_array = try!(files.as_array()
                                    .ok_or(DriveError {
                                        kind: DriveErrorType::JsonCannotConvertToArray,
                                        response: Some(resp_string.clone())
                                    }));
        Ok(files_array.clone())
    }

    pub fn get_files(&mut self, root_folder: (Vec<String>, &str)) -> Result<(), DriveError> {
        let files: json::Array = try!(self.get_file_list(&root_folder.1.clone())
            .or_else(|err: DriveError| {
                (match err.kind {
                    DriveErrorType::JsonInvalidAttribute => {
                        if let Some(resp_string) = err.response {
                            self.resolve_error(&resp_string)
                        } else {
                            Err(DriveError {
                                kind: DriveErrorType::Tester,
                                response: None
                            })
                        }
                    },
                    _ => Err(err)
                })
                .and_then(|_| {
                    println!("resolved error, so trying to redownload file list");
                    self.get_file_list(&root_folder.1.clone())
                })
        }));

        let mut dir_builder = DirBuilder::new();
        dir_builder.recursive(true);

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
                try!(dir_builder.create(new_path_str));
                // then recurse to retrieve children files
                try!(self.get_files((new_root_folder.clone(), &fr.id.clone())));
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
                        try!(remove_file(&metadata_path_str));
                        try!(self.download_and_save_file(new_root_folder, fr));
                    }
                };
            }
        }

        Ok(())
    }

    fn resolve_error(&mut self, resp_string: &str) -> Result<(), DriveError> {
        println!("attempting to resolve error response: {}", resp_string);

        let err_data = try!(Json::from_str(&resp_string));
        let err_obj  = try!(err_data.as_object()
                                    .ok_or(DriveError {
                                        kind: DriveErrorType::JsonObjectify,
                                        response: Some(resp_string.to_string())
                                    }));

        let error = try!(err_obj.get("error")
                                .ok_or(DriveError {
                                    kind: DriveErrorType::JsonInvalidAttribute,
                                    response: Some(resp_string.to_string())
                                }));
        let errors = try!(error.as_object()
                                    .ok_or(DriveError {
                                        kind: DriveErrorType::JsonObjectify,
                                        response: Some(resp_string.to_string())
                                    })
                                .and_then(|err_obj| err_obj.get("errors")
                                    .ok_or(DriveError {
                                        kind: DriveErrorType::JsonInvalidAttribute,
                                        response: Some(resp_string.to_string())
                                    }))
                                .and_then(|errors| errors.as_array()
                                    .ok_or(DriveError {
                                        kind: DriveErrorType::JsonInvalidAttribute,
                                            response: Some(resp_string.to_string())
                                    })));
        for i in errors {
            let mut decoder = Decoder::new(i.clone());
            let err: ErrorDetailsResponse = match Decodable::decode(&mut decoder) {
                Ok(err) => err,
                Err(error) => panic!("could not decode errorDetailsResponse, error: {}, attempted edr: {}", error, i)
            };

            if err.reason   == "authError"           &&
               err.message  == "Invalid Credentials" &&
               err.location == "Authorization" {
                let mut resp = try!(self.client.post("https://www.googleapis.com/oauth2/v3/token")
                                      .header(ContentType(Mime(TopLevel::Application, SubLevel::WwwFormUrlEncoded, vec![])))
                                      //.header(Host{hostname: "www.googleapis.com".to_owned(), port: None})
                                      .body(&format!("&client_id={}\
                                            &client_secret={}\
                                            &refresh_token={}\
                                            &grant_type=refresh_token", self.auth_data.client_id, self.auth_data.client_secret, self.auth_data.tr.refresh_token))
                                      .send());

                self.auth_data.tr.access_token = {
                    let mut ref_string = String::new();
                    try!(resp.read_to_string(&mut ref_string));
                    let ref_data = try!(Json::from_str(&ref_string));
                    let ref_obj  = try!(ref_data.as_object()
                                                .ok_or(DriveError {
                                                    kind: DriveErrorType::JsonObjectify,
                                                    response: Some(ref_string.clone())
                                                }));
                    try!(ref_obj.get("access_token")
                                .ok_or(DriveError {
                                    kind: DriveErrorType::JsonInvalidAttribute,
                                    response: Some(ref_string.clone())
                                })
                                .and_then(|acc| {
                                    acc.as_string()
                                       .ok_or(DriveError {
                                           kind: DriveErrorType::JsonInvalidAttribute,
                                           response: Some(ref_string)
                                       })
                                })).to_string()
                };

                // we'll open our access_cache file to read its current contents
                let mut f = try!(File::open(self.auth_data.cache_file_path.clone()));
                let mut access_string = String::new();
                try!(f.read_to_string(&mut access_string).and_then(|_| {
                    // if the current access file reads as it should, we'll copy the
                    // current token response and edit the   access_token field for later use
                    let mut tr: TokenResponse = match json::decode(&access_string) {
                        Ok(tr) => tr,
                        Err(error) => panic!()
                    };
                    tr.access_token = self.auth_data.tr.access_token.clone();
                    let tr_json = tr.to_json();
                    let tr_str = format!("{}", as_pretty_json(&tr_json));

                    // then we'll truncate the file and paste in the updated token response
                    // preserving all the other unused data
                    let mut f = try!(File::create(self.auth_data.cache_file_path.clone()));
                    f.write_all(tr_str.as_bytes())
                }))
            }
        }

        // for loop returns (), so a value for the function is needed
        Ok(())
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
            return Err(DriveError {
                kind: DriveErrorType::UnsupportedDocumentType,
                response: None
            })
        }

        // try to open the metadata file, if it already exists
        match File::open(metadata_path_str.clone()) {
            // the metadata file doesn't yet exist, so the file shouldn't exist either because the
            // two files are created at the same time: create_new_file(), so we'll download it
            Err(_) => {
                try!(self.create_new_file(fr, file_path));
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
                try!(self.verify_file_checksum(&fr, &fc.md5Checksum).or_else(|_| -> Result<String, DriveError> {
                    println!("updating metadata for {}", new_path_str);
                    // if the checksum fails, we need to redownload it
                    try!(self.create_new_file(fr.clone(), file_path.clone()));
                    Ok("".to_string())
                }));
            }
        };

        Ok(())
    }

    fn create_new_file(&mut self, fr: FileResponse, file_path: Vec<String>) -> Result<(), DriveError> {
        let new_path_str = convert_pathVec_to_pathString(file_path.clone());
        let metadata_path_str = convert_pathVec_to_metaData_pathStr(file_path.clone());

        let mut dotf = try!(File::create(metadata_path_str.clone()));

        // if the metadata file exists, but doesn't read (there was probably some corruption),
        // we'll checksum the file (if we have it) on the system, and compare it to the
        // server, if all is OK, create a new metadata file
        try!(get_file_checksum(file_path.clone()).or_else(|_| {
            println!("no file checksum yet");
            Err(DriveError {
                kind: DriveErrorType::FailedToChecksumExistingFile,
                response: None
            })
        }).and_then(|current_file_checksum| {
            self.verify_file_checksum(&fr, &current_file_checksum).and_then(|resp_string| {
                try!(dotf.write_all(&resp_string.into_bytes()));
                Ok(()) // this Ok(()) is necessary for the sam reason that the Ok(()) at the bottom of the function is:
                       // a mixture of io::Result and another Result type in the try block
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
        let mut resp = try!(self.send_authorized_request((&format!(
                        "https://www.googleapis.com/drive/v3/files/{}\
                        ?fields=md5Checksum%2Csize"
                        , fr.id.clone()))));

        let mut resp_string = String::new();
        try!(resp.read_to_string(&mut resp_string));
        let fcr: FileCheckResponse = try!(json::decode(&resp_string));
        let same = checksum == &fcr.md5Checksum;
        if !same {
            println!("{} =? {}", checksum, fcr.md5Checksum);
            println!("same?: {:?}", same);
            println!("size: {}", fcr.size);
            return Err(DriveError {
                kind: DriveErrorType::FailedChecksum,
                response: Some(resp_string)
            })
        }

        Ok(resp_string)
    }

    fn send_authorized_request_to_json<T: Decodable>(&mut self, url: &str) -> Result<T, DriveError> {
        self.send_authorized_request(url).and_then(|mut resp| {
            read_json_to_type(&mut resp)
        })
    }

    fn send_authorized_request(&mut self, url: &str) -> Result<client::Response, DriveError> {
        let resp = try!(self.client.get(url)
             .header(Authorization(Bearer{token: self.auth_data.tr.access_token.clone()}))
             .send());

        Ok(resp)
    }
}

fn get_file_checksum(file_path: Vec<String>) -> Result<String, DriveError> {
    let mut f = try!(File::open(convert_pathVec_to_pathString(file_path)));
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

pub fn request_new_access_code(c: &Client) -> Result<TokenResponse, DriveError> {
    // the space after client_id={} is necessary to seperate the link from the rest of the
    // prompt
    println!("Visit https://accounts.google.com/o/oauth2/v2/auth\
                    ?scope=email%20profile%20https://www.googleapis.com/auth/drive\
                    &redirect_uri=urn:ietf:wg:oauth:2.0:oob\
                    &response_type=code\
                    &client_id={} \
             to receive the access code.", CLIENT_ID);
    let mut code_string = String::new();
    try!(io::stdin().read_line(&mut code_string));

    let mut resp = c.post("https://accounts.google.com/o/oauth2/token")
        .header(ContentType(Mime(TopLevel::Application, SubLevel::WwwFormUrlEncoded, vec![])))
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
    try!(resp.read_to_string(&mut resp_string));
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
