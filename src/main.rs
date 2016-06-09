extern crate hyper;
extern crate mime;
extern crate rustc_serialize;
extern crate trie;
extern crate url;
extern crate itertools;
extern crate crypto;

use hyper::{Client, Server, server};
use hyper::header::{Host, ContentType, Authorization, Bearer, Range};
use mime::{Mime, TopLevel, SubLevel};
use rustc_serialize::json::{Json, ToJson, Decoder};
use rustc_serialize::{json, Decodable};
use trie::Trie;
use itertools::Itertools;

use std::io::prelude::*;
use std::io;
use std::fs::{File, DirBuilder};
use std::collections::BTreeMap;

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
        // All standard types implement `to_json()`, so use it
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
}

struct FileTree {
    tree: Trie<String, FileResponse>,
    client: Client,
    auth_data: AuthData,
}

fn main() {
    let c = Client::new();

    let file_tree = Trie::<String, FileResponse>::new();

    let tr: TokenResponse = match File::open("access") {
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
            match File::create("access") {
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
            client_id: "460434421766-0sktb0rkbvbko8omj8vhu8vv83giraao.apps.googleusercontent.com".to_owned(),
            client_secret: "m_ILEPtnZI53tXow9hoaabjm".to_owned(),
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
            self.tree.insert(new_root_folder.clone(), fr.clone());
    
            // convert the path vector to a string for file/folder creation in the system filesystem
            let new_path_str = new_root_folder.iter()
                .intersperse(&"/".to_owned())
                .fold("".to_owned(), |acc, ref filename| acc + &filename.clone());
    
            if fr.mimeType.clone() == "application/vnd.google-apps.folder" {
                println!("getting the next directory's files, {}", new_path_str.clone());
                // create the directory in the system filesystem
                dir_builder.create(new_path_str);
                // then recurse to retrieve children files
                self.get_files((new_root_folder.clone(), fr.id.clone()));
            } else {
            // we're working with a file, not a folder, so we need to save it to the system
                println!("saving file, {}", new_path_str.clone() + "/" + &fr.name.clone());
    
                // try to open the file, if it already exists
                match File::open(new_path_str.clone()) {
                    Ok(f) => {
                        //let mut maybe_resp = c.get(&format!("https://www.googleapis.com/drive/v3/files/{}\
                                                            //?fields=md5Checksum%2Csize", fr.id.clone()))
                                              //.header(Authorization(Bearer{token: access_token.clone()}))
                                       //       //.header(Range::bytes(0,500))
                                              //.send();
                        //let mut resp = match maybe_resp {
                            //Ok(resp) => resp,
                            //Err(error) => {
                                //println!("error when receiving response during file download, {}", error);
                                //continue;
                                //resolve_error(&resp_string);
                            //}
                        //};
    
                        //let mut resp_string = String::new();
                        //let mut resp_bytes = Vec::<u8>::new();
                        //resp.read_to_end(&mut resp_bytes);
                        //resp.read_to_string(&mut resp_string);
                        //println!("{}", resp_string);
                        //let fcr: FileCheckResponse = json::decode(&resp_string).unwrap();
                        //let dotf = match File::open(".".to_owned() + &new_path_str.clone()) {
                            //Ok(dotf) => dotf,
                            //Err(error) => File::create(".".to_owned() + &new_path_str.clone()).unwrap()
                        //};
    
                        //let mut fc_string = String::new();
                        //dotf.read_to_string(&mut fc_string);
                        //match json::decode(&fc_string) {
                            //Ok(fc) => {
                                //if (fc as FileCheckResponse).md5Checksum != fcr.md5Checksum.clone() {
                                    //println!("updating metadata for {}", new_path_str);
                                //}
                            //}
                            //Err(error) => {
                                //println!("reading the filecheck from {} failed, error: {:?}", new_path_str.clone(), error);
                                //dotf.write_all(&resp_bytes);
                            //}
                        //};
                    },
                    // the file doesn't yet exist, so we need to download it
                    Err(_) => {
                        let mut f = File::create(new_path_str).unwrap();
    
                        let mut maybe_resp = self.client.get(&format!("https://www.googleapis.com/drive/v3/files/{}\
                                                            ?alt=media"
                                                    , fr.id.clone()))
                                              .header(Authorization(Bearer{token: self.auth_data.tr.access_token.clone()}))
                                              //.header(Range::bytes(0,500))
                                              .send();
                        let mut resp = match maybe_resp {
                            Ok(resp) => resp,
                            Err(error) => {
                                println!("error when receiving response during file download, {}", error);
                                continue
                            }
                        };
                        let mut resp_bytes = Vec::<u8>::new();
                        let mut resp_string = String::new();
                        resp.read_to_end(&mut resp_bytes);
                        resp.read_to_string(&mut resp_string);
                        println!("{:?}", resp);
                        println!("{}", resp_string);
    
                        f.write_all(&resp_bytes);
                    }
                };
            }
        }
    }

    fn resolve_error(&mut self, resp_string: &String) -> io::Result<()> {
        let err_data = match Json::from_str(&resp_string) {
            Ok(fr) => fr,
            Err(error) => panic!("cannot read string as Json; invalid response, {}", error)
        };
        let err_obj  = match err_data.as_object() {
            Some(fr) => fr,
            None => panic!("JSON data returned was not an objcet")
        };
    
        match err_obj.get("error") {
            Some(error) => match error.as_object().unwrap().get("errors").unwrap().as_array() {
                Some(errors) => for i in errors {
                    let mut decoder = Decoder::new(i.clone());
                    let err: ErrorDetailsResponse = match Decodable::decode(&mut decoder) {
                        Ok(err) => err,
                        Err(error) => panic!("could not decode fileResponse, error: {}, attempted fr: {}", error, i)
                    };
    
                    if err.reason   == "authError"           &&
                       err.message  == "Invalid Credentials" &&
                       err.location == "Authorization" {
                        let mut resp = self.client.get(&format!("www.googleapis.com/oauth2/v3/token\
                                                   &client_id={}\
                                                   &client_secret={}\
                                                   &refresh_token={}\
                                                   &grant_type=refresh_token", self.auth_data.client_id, self.auth_data.client_secret, self.auth_data.tr.refresh_token))
                                              .send()
                                              .unwrap();
                        println!("{:?}", resp);
                        self.auth_data.tr.access_token = {
                            let mut resp_string = String::new();
                            resp.read_to_string(&mut resp_string);
                            let ref_data = match Json::from_str(&resp_string) {
                                Ok(fr) => fr,
                                Err(error) => panic!("cannot read string as Json; invalid response, {}", error)
                            };
                            let ref_obj  = match err_data.as_object() {
                                Some(fr) => fr,
                                None => panic!("JSON data returned was not an objcet")
                            };
                            ref_obj.get("access_token").unwrap().as_string().unwrap().to_owned()
                        }
                    }
                },
                None => panic!("the errors attribute was not a valid JSON array")
            },
            None => panic!("the response given to resolve_error() did not contain an error attribute")
        };
    
        Ok(())
    }
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
               &client_id=
               &client_secret=
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
