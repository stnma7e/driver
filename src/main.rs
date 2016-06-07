extern crate hyper;
extern crate mime;
extern crate rustc_serialize;

use hyper::{Client, Server, server};
use hyper::header::{Host, ContentType};
use mime::{Mime, TopLevel, SubLevel};
use rustc_serialize::json::{Json, ToJson};
use rustc_serialize::json;

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

fn main() {
    let c = Client::new();

    let tr: TokenResponse = match File::open("access") {
        // access file exists, so we can just use the access code that's stored in the file
        Ok(mut handle) => {
            let mut access_string = String::new();
            if let Ok(_) = handle.read_to_string(&mut access_string) {
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
                    let (tr, res_string) = request_new_access_code(&c);
                    handle.write_all(res_string.as_bytes());
                    tr
                }
                Err(error) => panic!("error when creating access file: {}", error),
            }
        }
    };

    println!("{:?}", tr);
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

    let mut res = c.post("https://accounts.google.com/o/oauth2/token")
        .header(ContentType(Mime(TopLevel::Application, SubLevel::WwwFormUrlEncoded, vec![])))
        .header(Host{hostname: "www.googleapis.com".to_owned(), port: None})
        .body(&format!("code={}&\
               client_id=
               client_secret=
               redirect_uri=urn:ietf:wg:oauth:2.0:oob&\
               grant_type=authorization_code"
            , code_string))
        .send()
        .unwrap();
    print!("{}\n{}\n{}\n\n", res.url, res.status, res.headers);

    let mut res_string = String::new();
    res.read_to_string(&mut res_string);
    let tr: TokenResponse = json::decode(&res_string).unwrap();
    (tr, res_string)
}
