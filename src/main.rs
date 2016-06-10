extern crate hyper;
extern crate mime;
extern crate rustc_serialize;
extern crate trie;
extern crate url;
extern crate itertools;
extern crate crypto;

use hyper::{Client, Server, client};
use hyper::header::{Host, ContentType, Authorization, Bearer, Range};
use mime::{Mime, TopLevel, SubLevel};
use rustc_serialize::json::{Json, ToJson, Decoder, as_pretty_json};
use rustc_serialize::{json, Decodable};
use trie::Trie;
use itertools::Itertools;
use crypto::md5::Md5;
use crypto::digest::Digest;

use std::io::prelude::*;
use std::io;
use std::fs::{File, DirBuilder, remove_file};
use std::collections::BTreeMap;
use std::error::Error;

#[derive (RustcDecodable, Debug, Clone)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u32,
    id_token: String,
    refresh_token: String,
}

impl TokenResponse {
    fn new() -> TokenResponse {
        TokenResponse {
            access_token: String::new(),
            token_type: String::new(),
            expires_in: 0,
            id_token: String::new(),
            refresh_token: String::new(),
        }
    }
}

impl ToJson for TokenResponse {
    fn to_json(&self) -> Json {
        let mut d = BTreeMap::new();
        d.insert("access_token".to_string(),  self.access_token.to_json());
        d.insert("token_type".to_string(),    self.token_type.to_json());
        d.insert("expires_in".to_string(),    self.expires_in.to_json());
        d.insert("id_token".to_string(),      self.id_token.to_json());
        d.insert("refresh_token".to_string(), self.refresh_token.to_json());
        Json::Object(d)
    }
}

#[derive (RustcDecodable, Debug, Clone)]
struct FileResponse {
    kind: String,
    id: String,
    name: String,
    mimeType: String,
}

impl ToJson for FileResponse {
    fn to_json(&self) -> Json {
        let mut d = BTreeMap::new();
        // All standard types implement `to_json()`, so use it
        d.insert("kind".to_string(),     self.kind.to_json());
        d.insert("id".to_string(),       self.id.to_json());
        d.insert("name".to_string(),     self.name.to_json());
        d.insert("mimeType".to_string(), self.mimeType.to_json());
        Json::Object(d)
    }
}

#[derive (RustcDecodable, Debug, Clone)]
struct ErrorDetailsResponse {
    domain: String,
    reason: String,
    message: String,
    locationType: String,
    location: String,
}

#[derive (RustcDecodable, Debug, Clone)]
struct FileCheckResponse {
    md5Checksum: String,
    size: String,
}

#[derive (Clone)]
struct AuthData {
    tr: TokenResponse,
    client_id: String,
    client_secret: String,
    // maybe this can be converted to a std::path::Path later?
    cache_file_path: String,
}

struct FileTree {
    tree: Trie<String, FileResponse>,
    client: Client,
    auth_data: AuthData,
}

fn main() {
    let c = Client::new();

    let cache_file = "access";
    let file_tree = Trie::<String, FileResponse>::new();

    let tr: TokenResponse = match File::open(cache_file) {
        // access file exists, so we can just use the access code that's stored in the file
        Ok(mut handle) => {
            let mut access_string = String::new();
            if let Ok(_) = handle.read_to_string(&mut access_string) {
                println!("{}", access_string);
                match json::decode(&access_string) {
                    Ok(tr) => tr,
                    Err(error) => panic!("reading the tokenResponse from the access file failed. is this really a TokenResponse?, {:?}", error),
                }
            } else {
                panic!("mklaf")
            }
        }

        // access file doesn't exist, so we need to poll the user for a new access code
        Err(error) => {
            match File::create(cache_file) {
                Ok(mut handle) => {
                    let (tr, resp_string) = request_new_access_code(&c);
                    println!("{}", resp_string);
                    handle.write_all(resp_string.as_bytes());
                    tr
                }
                Err(error) => panic!("error when creating access file: {}", error),
            }
        }
    };

    //println!("{:?}", tr);
    
    let mut ft = FileTree {
        tree: file_tree,
        client: Client::new(),
        auth_data: AuthData {
            tr: tr.clone(),
            client_id: "".to_owned(),
            client_secret: "".to_owned(),
            cache_file_path: "access".to_owned(),
        },
    };
    ft.get_files((vec!["root".to_string()], "root".to_string()));
    println!("{:?}", ft.tree);
    //println!("{:?}", file_tree.fetch)
}

impl FileTree {
    fn get_file_list(&mut self, root_folder: String) -> Result<json::Array, String> {
        let maybe_resp = self.client.get(&format!("https://www.googleapis.com/drive/v3/files\
                              ?corpus=domain\
                              &pageSize=100\
                              &q=%27{}%27+in+parents",
                              root_folder))
                          .header(Authorization(Bearer{token: self.auth_data.tr.access_token.clone()}))
                          .send();
        // check to make sure response is valid
        let mut resp = match maybe_resp {
            Ok(resp) => resp,
            Err(error) => return Err(format!("error in get request when receiving file list, {}", error))
        };
    
        // convert the response to a string, then to a JSON object
        let mut resp_string = String::new();
        resp.read_to_string(&mut resp_string);
        let fr_data = match Json::from_str(&resp_string) {
            Ok(fr) => fr,
            Err(error) => panic!("cannot read string as Json; invalid response, {}", error)
        };
        let fr_obj  = match fr_data.as_object() {
            Some(fr) => fr,
            None => panic!("JSON data returned by Drive was not an objcet")
        };
        
        //println!("{}", resp_string);

        match fr_obj.get("files") {
            Some(files) => match files.as_array() {
                Some(files_array) => Ok(files_array.clone()),
                None => panic!("the files attribute from the file list was not a valid JSON array")
            },
            // maybe the json returned was really an error response
            // so let's try to reauthenticate
            None => {
                println!("{}", resp_string);
                match self.resolve_error(&resp_string) {
                // if resolve_error did something (could be a re-authorization request, etc.), we can
                // try to get the file list again
                Ok(_) => self.get_file_list(root_folder),
                // otherwise resolve_error encountered some other error, and was not able to reslove
                // anything
                Err(error) => Err(format!("there was no files attribute in this response, {}; resolution failed with: {}", resp_string, error))
            }
            }
        }
    }

    fn get_files(&mut self, root_folder: (Vec<String>, String)) {
        let files = match self.get_file_list(root_folder.1.clone()) {
            Ok(files) => files,
            Err(error) => panic!(error)
        };
        let mut dir_builder = DirBuilder::new();
        dir_builder.recursive(true);
    
        // our file list is A-OK
        for i in files.iter() {
            // we'll try to decode each file's metadata JSON object in memory to a FileResponse struct
            let mut decoder = Decoder::new(i.clone());
            let fr: FileResponse = match Decodable::decode(&mut decoder) {
                Ok(fr) => fr,
                // whatever JSON array we received before has something in it that's not a FileResponse
                Err(error) => panic!("could not decode fileResponse, error: {}, attempted fr: {}", error, i)
            };
    
            // this will be our starting path when we add to the trie later
            let mut new_root_folder = root_folder.0.clone();
            // add the file we're working with to the path for the trie
            new_root_folder.push(fr.name.clone());
            let new_path_str = convert_pathVec_to_pathString(new_root_folder.clone());
            let metadata_path_str = convert_pathVec_to_metaData_pathStr(new_root_folder.clone());

            self.tree.insert(new_root_folder.clone(), fr.clone());
    
            if fr.mimeType.clone() == "application/vnd.google-apps.folder" {
                println!("getting the next directory's files, {}", new_path_str.clone());
                // create the directory in the system filesystem
                dir_builder.create(new_path_str);
                // then recurse to retrieve children files
                self.get_files((new_root_folder.clone(), fr.id.clone()));
            } else {
            // we're working with a file, not a folder, so we need to save it to the system
                println!("saving file, {}", new_path_str.clone());
                match self.download_and_save_file(new_root_folder.clone(), fr.clone()) {
                    Ok(_) => (),
                    Err(error) => {
                        println!("error when saving or downloading file: {}", error);
                        println!("deleting metadata, and trying a fresh save");
                        remove_file(&metadata_path_str);
                        self.download_and_save_file(new_root_folder, fr);
                    }
                };
            }
        }
    }

    fn resolve_error(&mut self, resp_string: &String) -> io::Result<()> {
        let err_data = match Json::from_str(&resp_string) {
            Ok(fr) => fr,
            Err(error) => panic!("cannot read string as Json; error: {}, invalid response: {}", error, resp_string)
        };
        let err_obj  = match err_data.as_object() {
            Some(fr) => fr,
            None => panic!("JSON data returned was not an objcet")
        };
    
        match err_obj.get("error") {
            Some(error) => match error.as_object().expect("not an object").get("errors").expect("no array").as_array() {
                Some(errors) => for i in errors {
                    let mut decoder = Decoder::new(i.clone());
                    let err: ErrorDetailsResponse = match Decodable::decode(&mut decoder) {
                        Ok(err) => err,
                        Err(error) => panic!("could not decode fileResponse, error: {}, attempted fr: {}", error, i)
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
                            ref_obj.get("access_token").expect("afsd").as_string().unwrap().to_owned()
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
                },
                None => panic!("the response given to resolve_error() had no array of errors")
            },
            None => panic!("the response given to resolve_error() did not contain an error attribute")
        };
    
        Ok(())
    }

    fn download_and_save_file(&mut self, file_path: Vec<String>, fr: FileResponse) -> Result<(), String> {
        let new_path_str = convert_pathVec_to_pathString(file_path.clone());
        let metadata_path_str = convert_pathVec_to_metaData_pathStr(file_path.clone());

        // try to open the metadata file, if it already exists
        match File::open(metadata_path_str.clone()) {
            Ok(mut dotf) => {
                // if it does, we'll read the file to see if the checksum matches what we have on
                // the server
                match read_json_to_type(&mut dotf).1 as Result<FileCheckResponse, String> {
                    Ok(fc) => {
                        if !self.verify_file_checksum(fr.clone(), &fc.md5Checksum).0 {
                            println!("updating metadata for {}", new_path_str);

                            // if the checksum fails, we need to redownload it
                            self.create_new_file(fr.clone(), file_path.clone());
                        }
                    }
                    // if the file exists, but doesn't read (this should be an error), but we'll
                    // checksum the file (if we have it) on the system, and compare it to the
                    // server, if all is OK, create a new metadata file
                    Err(error) => {
                        println!("reading the filecheck from {} failed, error: {:?}", new_path_str.clone(), error);
                        println!("creating new metadata file");
                        let mut dotf = File::create(metadata_path_str.clone()).unwrap();
                        // if the file exists, get the checksum
                        if let Ok(current_file_checksum) = get_file_checksum(file_path.clone()) {
                            let (equal, resp_string) = self.verify_file_checksum(fr.clone(), &current_file_checksum);
                            // if the checksum matches that from Drive, then file is downloaded
                            // correctly, and all we need to do is cache the checksum
                            if equal {
                                dotf.write_all(&resp_string.into_bytes());
                            // the file's checksum doesn't match the server's, so we need to
                            // re-download it
                            } else {
                                println!("checksum match failed, redownloading file {}", new_path_str);
                                self.create_new_file(fr, file_path);
                            }
                        // otherwise, the file doesn't exist yet, so just create it
                        } else {
                            self.create_new_file(fr, file_path);
                        }
                    }
                };
            },
            // the metadata file doesn't yet exist, so the file shouldn't exist either because the
            // two files are created at the same time: create_new_file(), so we'll download it
            Err(_) => {
                println!("creating new file with metadata: {}", metadata_path_str);
                self.create_new_file(fr, file_path);
            }
        };

        Ok(())
    }

    fn create_new_file(&mut self, fr: FileResponse, file_path: Vec<String>) -> Result<(), String> {
        let new_path_str = convert_pathVec_to_pathString(file_path.clone());
        let metadata_path_str = convert_pathVec_to_metaData_pathStr(file_path.clone());
        let mut dotf = File::create(metadata_path_str.clone()).expect("no metadata file could be created");

        println!("creating new metadata file: {}", metadata_path_str);

        let mut f = File::create(new_path_str).unwrap();

        let mut resp = self.send_authorized_request(
            format!("https://www.googleapis.com/drive/v3/files/{}\
                    ?alt=media", fr.id.clone()));

        let mut resp_string = String::new();
        resp.read_to_string(&mut resp_string);
        f.write_all(&resp_string.clone().into_bytes());;

        println!("{}", resp_string.clone());

        let md5_result = {
            let mut md5 = Md5::new();
            md5.input_str(&resp_string);
            md5.result_str()
        };

        let (checksum_result, resp_string) = self.verify_file_checksum(fr, &md5_result);
        if checksum_result {
            dotf.write_all(&resp_string.into_bytes());
            Ok(())
        } else {
            Err("downloaded checksums did not match".to_owned())
        }
    }

    fn verify_file_checksum(&mut self, fr: FileResponse, checksum: &String) -> (bool, String) {
        let (resp_string, resp) = self.send_authorized_request_to_json(
            format!("https://www.googleapis.com/drive/v3/files/{}\
                     ?fields=md5Checksum%2Csize", fr.id.clone()));
        let fcr: FileCheckResponse = match resp {
            Ok(fcr) => fcr,
            Err(error) => {
                println!("error when decoding filecheckresponse, {}", error);
                self.resolve_error(&resp_string);
                match self.send_authorized_request_to_json(
                    format!("https://www.googleapis.com/drive/v3/files/{}\
                             ?fields=md5Checksum%2Csize", fr.id.clone())).1 {
                    Ok(fcr) => fcr,
                    Err(error) => {
                        println!("no valid response from Drive, error: {}", error);
                        return (false, resp_string.clone())
                    }
                }
            }
        };

        let same = checksum == &fcr.md5Checksum;
        println!("{} =? {}", checksum, fcr.md5Checksum);
        println!("same?: {:?}", same);
        if !same {
            println!("size: {}", fcr.size);
        }
        (same, resp_string)
    }

    fn send_authorized_request_to_json<T: Decodable>(&mut self, url: String) -> (String, Result<T, String>) {
        read_json_to_type(&mut self.send_authorized_request(url))
    }

    fn send_authorized_request(&mut self, url: String) -> client::Response {
        let mut maybe_resp = self.client.get(&url)
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

    f.read_to_string(&mut f_str);

    let mut md5 = Md5::new();
    md5.input_str(&f_str);
    Ok(md5.result_str())
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

fn read_json_to_type<J: Read, T: Decodable>(json: &mut J) -> (String, Result<T, String>) {
        let mut resp_string = String::new();
        json.read_to_string(&mut resp_string);
        (resp_string.clone(), match json::decode(&resp_string) {
            Ok(t) => Ok(t),
            Err(error) => Err(format!("{}", error))
        })
}

fn request_new_access_code(c: &Client) -> (TokenResponse, String) {
    // the space after googleusercontent.com is necessary to seperate the link from the rest of the
    // prompt
    println!("Visit https://accounts.google.com/o/oauth2/v2/auth\
                    ?scope=email%20profile%20https://www.googleapis.com/auth/drive\
                    &redirect_uri=urn:ietf:wg:oauth:2.0:oob\
                    &response_type=code\
                    &client_id=
             to receive the access code.");
    let mut code_string = String::new();
    match io::stdin().read_line(&mut code_string) {
        Ok(_) => {}
        Err(error) => println!("error: {}", error),
    }

    let mut resp = c.post("https://accounts.google.com/o/oauth2/token")
        .header(ContentType(Mime(TopLevel::Application, SubLevel::WwwFormUrlEncoded, vec![])))
        //.header(Host{hostname: "www.googleapis.com".to_owned(), port: None})
        .body(&format!("code={}\
               &client_id=\
               &client_secret=\
               &redirect_uri=urn:ietf:wg:oauth:2.0:oob\
               &grant_type=authorization_code"
               , code_string))
        .send()
        .unwrap();
    print!("{}\n{}\n{}\n\n", resp.url, resp.status, resp.headers);

    let mut resp_string = String::new();
    resp.read_to_string(&mut resp_string);
    let tr: TokenResponse = json::decode(&resp_string).unwrap();
    (tr, resp_string)
}

fn convert_pathVec_to_pathString(path_vec: Vec<String>) -> String {
    // convert the path vector to a string for file/folder creation in the system filesystem
    path_vec.iter()
            .intersperse(&"/".to_owned())
            .fold("".to_owned(), |acc, ref filename| acc + &filename.clone())
}

