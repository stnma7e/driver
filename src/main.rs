#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(unused_variables)]

extern crate hyper;
extern crate rustc_serialize;
extern crate driver;
extern crate fuse;

use hyper::{Client};
use rustc_serialize::json::{ToJson};
use rustc_serialize::{json};
use std::io::prelude::*;
use std::fs::{File};
use std::collections::hash_map::HashMap;

use driver::types::*;
use driver::filetree::*;

const CACHE_FILE: &'static str = "access";

fn main() {
    let c = Client::new();

    let tr = || -> Result<TokenResponse, DriveError> {
        let mut handle = try!(File::open(CACHE_FILE).map_err(From::from)
            .or_else(|err: std::io::Error| -> Result<std::fs::File, DriveError> {
                let mut handle = try!(File::create(CACHE_FILE));
                let tr = try!(request_new_access_code(&c));
                println!("{}", tr.clone().to_json().to_string());
                try!(handle.write_all(tr.to_json().to_string().as_bytes()));
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
    ft.current_inode += 1;

    || -> Result<_, DriveError> {
        ft.get_files(root_folder)
    }().expect("this shit fucked up");

    println!("{:?}", ft.files);
    println!("{:?}", ft.inode_map);

    fuse::mount(ft, &"root.2", &[]);
}
