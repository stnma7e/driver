#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(unused_variables)]

extern crate hyper;
extern crate rustc_serialize;
extern crate driver;
extern crate fuse;
extern crate uuid;
extern crate time;

use std::collections::hash_map::HashMap;
use std::path::Path;
use uuid::Uuid;
use fuse::FileAttr;

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

    let root_folder_id =  "0B7TtU3YsiIjTWjBOM0YwYkVBa1U";
    let mut fd = DriveFileDownloader::new(root_folder_uuid, root_folder_id.to_string(), Path::new(root_folder_name).to_owned())
        .expect("failure in reading access file");

    let mut ft = FileTree {
        inode_map: HashMap::new(),
        child_map: HashMap::new(),
        parent_map: HashMap::new(),
        current_inode: root_folder_inode,
        root_folder: root_folder_name,
        file_downloader: &mut fd,
    };

    let ts = time::now().to_timespec();
    ft.inode_map.entry(root_folder_inode).or_insert(FileData {
        id: root_folder_uuid.clone(),
        path: root_folder_path.to_owned(),
        parent_inode: root_folder_inode,
        attr: FileAttr {
            ino: root_folder_inode,
            size: 0,
            blocks: 0,
            atime: ts,
            mtime: ts,
            ctime: ts,
            crtime: ts,
            kind: FileType::Directory,
            perm: 0o777,
            nlink: 0,
            uid: 1000,
            gid: 1000,
            rdev: 0,
            flags: 0,
        },
        source_data: SourceData::CreatedFile,
    });
    ft.child_map.entry(ft.current_inode).or_insert(Vec::new());
    ft.parent_map.entry(ft.current_inode).or_insert(ft.current_inode);
    ft.current_inode += 1;

    ft.get_files(root_folder_path, &root_folder_uuid, root_folder_inode)
        .expect("this shit fucked up");

    println!("{:?}\n", ft.inode_map);
    println!("{:?}", ft.child_map);

    fuse::mount(ft, &"root.2", &[]);
}
