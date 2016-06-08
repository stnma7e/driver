extern crate hyper;
extern crate mime;
extern crate rustc_serialize;
extern crate trie;
extern crate url;

use hyper::{Client, Server, server};
use hyper::header::{Host, ContentType, Authorization, Bearer, Range};
use mime::{Mime, TopLevel, SubLevel};
use rustc_serialize::json::{Json, ToJson, Decoder};
use rustc_serialize::{json, Decodable};
use trie::Trie;

use std::io::prelude::*;
use std::io;
use std::fs::File;
use std::collections::BTreeMap;

#[derive (RustcDecodable, Debug)]
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

fn main() {
    let c = Client::new();

    let mut file_tree = Trie::<String, FileResponse>::new();

    let tr: TokenResponse = match File::open("access") {
        // access file exists, so we can just use the access code that's stored in the file
        Ok(mut handle) => {
            let mut access_string = String::new();
            if let Ok(_) = handle.read_to_string(&mut access_string) {
                println!("{}", access_string);
                let tr: TokenResponse = json::decode(&access_string).unwrap();
                tr
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

    {
        let mut resp = c.get("https://www.googleapis.com/drive/v3/files/?alt=media")
                      .header(Authorization(Bearer{token: tr.access_token.clone()}))
                      //.header(Range::bytes(0,500))
                      .send()
                      .expect("adsf");
        let mut resp_bytes = Vec::<u8>::new();
        let mut resp_string = String::new();
        resp.read_to_end(&mut resp_bytes);
        resp.read_to_string(&mut resp_string);
        println!("{:?}", resp);
        println!("{}", resp_string);
        let mut f = File::create("blah.jpg").unwrap();
        f.write_all(&resp_bytes);
    }

    get_files(&mut file_tree, &c, tr.access_token, (vec!["root".to_string()], "root".to_string()));
    println!("{:?}", file_tree);
    //println!("{:?}", file_tree.fetch)
}

fn get_files(file_tree: &mut Trie<String, FileResponse>, c: &Client, access_token: String, root_folder: (Vec<String>, String)) {
    let mut resp = c.get(&format!("https://www.googleapis.com/drive/v3/files\
                          ?corpus=domain\
                          &pageSize=100\
                          &q=%27{}%27+in+parents",
                          root_folder.1.clone()))
                      .header(Authorization(Bearer{token: access_token.clone()}))
                      .send()
                      .unwrap();
    let mut resp_string = String::new();
    resp.read_to_string(&mut resp_string);
    let fr_data = Json::from_str(&resp_string).unwrap();
    let fr_obj  = fr_data.as_object().unwrap();
    
    //println!("{}", resp_string);

    for i in (fr_obj.get("files").unwrap().as_array().unwrap()).iter() {
        let mut decoder = Decoder::new(i.clone());
        let fr: FileResponse = Decodable::decode(&mut decoder).unwrap();
        if fr.mimeType.clone() == "application/vnd.google-apps.folder" {
            //println!("\n\n\ngetting the next directory's files, {}\n\n\n", fr.id.clone());
            get_files(file_tree, c, access_token.clone(), (root_folder.0.clone(), fr.id.clone()));
        }
        let mut new_root_folder = root_folder.0.clone();
        new_root_folder.push(fr.name.clone());
        file_tree.insert(new_root_folder, fr);
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
        .body(&format!("code={}&\
               client_id=
               client_secret=
               redirect_uri=urn:ietf:wg:oauth:2.0:oob&\
               grant_type=authorization_code"
            , code_string))
        .send()
        .unwrap();
    print!("{}\n{}\n{}\n\n", resp.url, resp.status, resp.headers);

    let mut resp_string = String::new();
    resp.read_to_string(&mut resp_string);
    let tr: TokenResponse = json::decode(&resp_string).unwrap();
    (tr, resp_string)
}
