#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(unused_variables)]

extern crate hyper;
extern crate rustc_serialize;
extern crate driver;
extern crate fuse;
extern crate uuid;

use std::collections::hash_map::HashMap;
use std::path::Path;
use uuid::Uuid;

use driver::types::*;
use driver::filetree::*;
use driver::drive::*;

fn main() {
//    let root_folder = (vec![], "0B7TtU3YsiIjTTS1oUE5wZFpsYVk");
    let root_folder_name = "rot";
    let root_folder_path = Path::new(root_folder_name);
    let root_folder_id =  "0B7TtU3YsiIjTWjBOM0YwYkVBa1U";
    let root_folder_uuid = Uuid::new_v4();
    let root_folder_inode = 1;
//    let root_folder = (vec!["rot".to_string()], "0B7TtU3YsiIjTeHJGR1VKMHB3cWs");

    let mut fd = DriveFileDownloader::new(root_folder_uuid, root_folder_id.to_string())
                    .expect("failure in reading access file");

    let mut ft = FileTree {
        files: HashMap::new(),
        inode_map: HashMap::new(),
        child_map: HashMap::new(),
        parent_map: HashMap::new(),
        current_inode: 1,
        root_folder: root_folder_name,
        file_downloader: &mut fd,
    };

    ft.files.entry(root_folder_uuid).or_insert(ft.current_inode);
    ft.child_map.entry(ft.current_inode).or_insert(Vec::new());
    ft.parent_map.entry(ft.current_inode).or_insert(ft.current_inode);
    ft.current_inode += 1;

    || -> Result<_, DriveError> {
        ft.get_files(root_folder_path, &root_folder_uuid, root_folder_inode)
    }().expect("this shit fucked up");

    println!("{:?}\n", ft.files);
    println!("{:?}\n", ft.inode_map);
    println!("{:?}", ft.child_map);

    fuse::mount(ft, &"root.2", &[]);
}
