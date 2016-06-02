extern crate hyper;
extern crate mime;

use hyper::Client;
use hyper::header::{Host, ContentType};
use mime::{Mime, TopLevel, SubLevel};

use std::io::Read;

fn main() {
    let c = Client::new();
    let mut res = c.post("https://accounts.google.com/o/oauth2/device/code")
        .header(ContentType(Mime(TopLevel::Application, SubLevel::WwwFormUrlEncoded, vec![])))
        .header(Host{hostname: "accounts.google.com".to_owned(), port: None})
        .body("")
        .send()
        .unwrap();
    print!("{}\n{}\n{}\n\n", res.url, res.status, res.headers);

    let mut res_string = String::new();
    res.read_to_string(&mut res_string);
    println!("{}", res_string);
}
