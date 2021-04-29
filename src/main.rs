#![feature(seek_stream_len)]

mod archive;

use archive::{Archive, VersionNumber};

fn main() {

    let mut archive = Archive::new("E:\\Software Projects\\IntelliJ\\gud_archive\\test");

    archive.create();

    let mut appender = archive.appender(VersionNumber{number: 133}, String::from("Initial things"));

    use std::env::{current_dir, set_current_dir};

    let current = current_dir().unwrap();

    set_current_dir("E:\\Software Projects\\IntelliJ\\gud_archive").unwrap();

    appender.append_snapshot("a.txt");
    appender.append_snapshot("b.txt");
    appender.finish();

    set_current_dir(current).unwrap();

    let mut reader = archive.reader();

    let mut s = Vec::new();

    let mut taken = reader.file(0, "a.txt", & mut s).unwrap();

    println!("{}", String::from_utf8(s).unwrap());


}
