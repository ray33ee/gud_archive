use std::fs::{File, OpenOptions};
use std::path::{PathBuf, Path};
use serde::{Serialize, Deserialize};
use std::time::SystemTime;
use std::io::{Read, Write, Seek, SeekFrom};
use std::collections::{HashSet, HashMap};
use std::borrow::Borrow;

#[derive(Serialize, Deserialize)]
enum Contents {
    Snapshot,
    Patch
}

#[derive(Serialize, Deserialize)]
enum FileType {
    File,
    Directory,
    SystemLink,
}

#[derive(Serialize, Deserialize)]
struct Metadata {
    file_type: FileType,
    len: u64,
    read_only: bool,
    modified: Option<SystemTime>,
    accessed: Option<SystemTime>,
    created: Option<SystemTime>,
}

#[derive(Serialize, Deserialize)]
struct FileHeader {
    metadata: Metadata,
    path: PathBuf,
    contents: Contents,
}

#[derive(Serialize, Deserialize)]
struct VersionNumber {
    pub number: u64
}

#[derive(Serialize, Deserialize)]
struct VersionHeader {
    files: HashMap<PathBuf, u64>,
    number: VersionNumber,
    message: String,
    previous: Option<u64>, //Offset to previous version
}

impl VersionHeader {
    fn new(number: VersionNumber, message: String, previous: Option<u64>) -> Self {
        VersionHeader {
            files: HashMap::new(),
            number,
            message,
            previous
        }
    }

    fn insert(& mut self, path: PathBuf, offset: u64) {
        self.files.insert(path, offset);
    }
}

#[derive(Serialize, Deserialize)]
struct VersionDirectory {
    directory: Vec<u64>,
}

impl VersionDirectory {
    pub fn new() -> Self {
        VersionDirectory {
            directory: Vec::new(),
        }
    }

    pub fn directory(& self) -> & [u64] { self.directory().as_ref() }

    pub fn add(& mut self, offset: u64) { self.directory.push(offset); }
}

impl Metadata {
    fn new(path: & Path) -> Self {

        let metadata = std::fs::metadata(path).unwrap();

        Metadata {
            file_type: if metadata.is_file() { FileType::File } else { FileType::Directory },
            len: metadata.len(),
            read_only: metadata.permissions().readonly(),
            modified: if let std::io::Result::Ok(date_time) = metadata.modified() { Some(date_time) } else { None },
            accessed: if let std::io::Result::Ok(date_time) = metadata.accessed() { Some(date_time) } else { None },
            created: if let std::io::Result::Ok(date_time) = metadata.created() { Some(date_time) } else { None },
        }
    }
}

impl FileHeader {
    fn new(path: & Path, contents: Contents) -> Self {
        let metadata = Metadata::new(path);
        let path = PathBuf::from(path);

        FileHeader {
            metadata,
            path,
            contents
        }

    }
}

struct AppendArchive {
    fp: File,
    backup_directory: VersionDirectory, //A backup of the version directory
    version_header: VersionHeader,
}

impl AppendArchive {
    //Open file
    fn new(archive_path: & Path, number: VersionNumber, message: String) -> Self {
        let mut fp = OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .append(true)
            .open(archive_path).unwrap();

        //Get the first u64 (version directory offset)
        let mut number_buffer = [0u8; 8];

        let offset = bincode::deserialize::<u64>(number_buffer.as_mut()).unwrap();

        //seek to this offset
        fp.seek(SeekFrom::Start(offset));

        //make a backup of the data from offset to EOF
        let mut backup_directory = Vec::new();

        fp.read(& mut backup_directory);

        let mut backup_directory = bincode::deserialize::<VersionDirectory>(backup_directory.as_slice()).unwrap();

        //Get the offset of the very last entry (store in self.previous)
        let previous = backup_directory.directory().last().borrow().cloned();

        //Seek back to offset, so that future appends overwite the old version directory
        fp.seek(SeekFrom::Start(offset));

        //Add offset to version directory
        //backup_directory.add(offset);

        let version_header = VersionHeader::new(number, message, previous);

        AppendArchive {
            fp,
            backup_directory,
            version_header,
        }

    }

    //Append Version to archive, sort out directory and the directory offset
    fn append_snapshot(& mut self, path: PathBuf) {

        //Save the position of the header
        let position = self.fp.seek(SeekFrom::Current(0)).unwrap();

        //Create the file header for the file entry
        let header = FileHeader::new(path.as_path(), Contents::Snapshot);

        //Write the header to the archive
        self.fp.write_all(bincode::serialize(&header).unwrap().as_slice());

        //Open the file to append
        let mut fp = OpenOptions::new()
            .read(true)
            .open(path.as_path()).unwrap();

        //Copy the file into the archive
        std::io::copy(& mut fp, & mut self.fp);

        //Add position of the header to list
        self.version_header.insert(path, position);

    }

    fn finish(& mut self) {

        let version_header_offset = self.fp.seek(SeekFrom::Current(0)).unwrap();

        //Append the version header
        let raw_header = bincode::serialize(&self.version_header).unwrap();

        self.fp.write_all(raw_header.as_ref());

        //Get the size of the file (offset version directory)
        let directory_offset = self.fp.seek(SeekFrom::End(0)).unwrap();

        //Add the new entry in the version directory
        self.backup_directory.add(version_header_offset);

        //append the new version directory
        let raw_directory = bincode::serialize(&self.backup_directory).unwrap();

        self.fp.write_all(raw_directory.as_ref());

        //set the first u64 to the offset of the version directory
        self.fp.seek(SeekFrom::Start(0));

        let raw_offset = bincode::serialize(&directory_offset).unwrap();
        self.fp.write_all(&raw_offset);
    }

}

impl Drop for AppendArchive {
    fn drop(&mut self) {
        self.finish();
    }
}

struct Archive {
    path: PathBuf,
}

impl Archive {
    pub fn new(path: & Path) -> Self {
        Archive {
            path: PathBuf::from(path),
        }
    }

    pub fn create(& self) {
        let mut fp = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path).unwrap();

        //insert a 0u64 at the beginning
        fp.write_all([0u8; 8].as_ref());

        //Insert an empty VersionDirectory after
        fp.write_all(bincode::serialize(&VersionDirectory::new()).unwrap().as_slice());
    }

    pub fn appender(& mut self, number: VersionNumber, message: String) -> AppendArchive {
        AppendArchive::new(&self.path, number, message)
    }

    /*pub fn reader(& mut self) -> ReadArchive {
        ReadArchive::new(&self.path)
    }*/
}

fn main() {


    let mut fp = OpenOptions::new()
        .read(true)
        .open("E:\\Will\\Documents\\SPRITE1.txt").unwrap();

    println!("{}", fp.seek(SeekFrom::End(0)).unwrap());
}
