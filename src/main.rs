extern crate hyper;
extern crate mime;

use hyper::{Client, Server};
use hyper::server::{Request, Response};
use hyper::header::{Host, ContentType};
use mime::{Mime, TopLevel, SubLevel};

use std::io::Read;

fn get_auth_token(req: Request, res: Response) {
    
}

fn main() {
    let c = Client::new();
    let mut res = c.post("https://accounts.google.com/o/oauth2/token")
        .header(ContentType(Mime(TopLevel::Application, SubLevel::WwwFormUrlEncoded, vec![])))
        .header(Host{hostname: "www.googleapis.com".to_owned(), port: None})
        .body("code=4/OWxxjB3qQ6c-w-t7f7zpJ3a-HRs9_jIaSC7meI22oPE&\
               client_id=460434421766-0sktb0rkbvbko8omj8vhu8vv83giraao.apps.googleusercontent.com&\
               client_secret=m_ILEPtnZI53tXow9hoaabjm&\
               redirect_uri=urn:ietf:wg:oauth:2.0:oob&\
               grant_type=authorization_code")
        .send()
        .unwrap();
    print!("{}\n{}\n{}\n\n", res.url, res.status, res.headers);

    let mut res_string = String::new();
    res.read_to_string(&mut res_string);
    println!("{}", res_string);
}

// https://accounts.google.com/o/oauth2/v2/auth?scope=email%20profile&redirect_uri=urn:ietf:wg:oauth:2.0:oob&response_type=code&client_id=460434421766-0sktb0rkbvbko8omj8vhu8vv83giraao.apps.googleusercontent.com
