

use std::fs::{File, OpenOptions, read};
use std::path::{PathBuf, Path};
use serde::{Serialize, Deserialize};
use std::time::SystemTime;
use std::io::{Read, Write, Seek, SeekFrom, Take};
use std::collections::{HashMap};
use std::borrow::Borrow;
use lzma_rs::{lzma_compress, lzma_decompress};

#[derive(Serialize, Deserialize, Debug, Clone)]
enum Contents {
    Snapshot,
    Patch
}

#[derive(Serialize, Deserialize, Debug, Clone)]
enum FileType {
    File,
    Directory,
    SystemLink,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Metadata {
    file_type: FileType,
    len: u64,
    read_only: bool,
    modified: Option<SystemTime>,
    accessed: Option<SystemTime>,
    created: Option<SystemTime>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct FileHeader {
    compressed_size: u64,
    metadata: Metadata,
    path: PathBuf,
    contents: Contents,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VersionNumber {
    pub number: u64
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct VersionHeader {
    files: HashMap<PathBuf, u64>,
    number: VersionNumber,
    message: String,
}

impl VersionHeader {
    fn new(number: VersionNumber, message: String) -> Self {
        VersionHeader {
            files: HashMap::new(),
            number,
            message,
        }
    }

    fn insert(& mut self, path: & Path, offset: u64) {
        self.files.insert(PathBuf::from(path), offset);
    }
}

#[derive(Debug, Clone)]
struct Version {
    pub files: HashMap<PathBuf, (u64, FileHeader)>,
    pub number: VersionNumber,
    pub message: String,
}

impl Version {
    fn new(files: HashMap<PathBuf, (u64, FileHeader)>, number: VersionNumber, message: String) -> Self {

        Version {
            files,
            number,
            message,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct VersionDirectory {
    directory: Vec<u64>,
}

impl VersionDirectory {
    pub fn new() -> Self {
        VersionDirectory {
            directory: Vec::new(),
        }
    }

    pub fn directory(& self) -> & [u64] { self.directory.as_ref() }

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
            compressed_size: 0,
            metadata,
            path,
            contents
        }

    }
}

pub struct Archive {
    path: PathBuf,
}

impl Archive {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Archive {
            path: PathBuf::from(path.as_ref()),
        }
    }

    pub fn create(& self) {
        let mut fp = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path).unwrap();

        //insert a 8u64 at the beginning
        bincode::serialize_into(&fp, &8u64);

        //Insert an empty VersionDirectory after
        bincode::serialize_into(&fp, &VersionDirectory::new());
    }

    pub fn appender(& mut self, number: VersionNumber, message: String) -> AppendArchive {
        AppendArchive::new(&self.path, number, message)
    }

    pub fn reader(& mut self) -> ReadArchive {
        ReadArchive::new(&self.path)
    }
}

pub struct AppendArchive {
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
            .open(archive_path).unwrap();

        //Get the first u64 (version directory offset)
        let offset = bincode::deserialize_from::<_, u64>(&fp).unwrap();

        //seek to this offset
        fp.seek(SeekFrom::Start(offset)).unwrap();

        //make a backup of the data from offset to EOF
        let backup_directory = bincode::deserialize_from::<_, VersionDirectory>(&fp).unwrap();

        //Seek back to offset, so that future appends overwite the old version directory
        fp.seek(SeekFrom::Start(offset)).unwrap();

        let version_header = VersionHeader::new(number, message);

        AppendArchive {
            fp,
            backup_directory,
            version_header,
        }

    }

    //Append Version to archive, sort out directory and the directory offset
    pub fn append_snapshot<P: AsRef<Path>>(& mut self, path: P) {

        if path.as_ref().is_absolute() {
            panic!("Appended path MUST be relative.")
        }

        //Save the position of the header
        let position = self.fp.stream_position().unwrap();

        //Create the file header for the file entry
        let header = FileHeader::new(path.as_ref(), Contents::Snapshot);

        //Write the header to the archive
        bincode::serialize_into(&self.fp, &header).unwrap();

        //Open the file to append
        let mut fp = OpenOptions::new()
            .read(true)
            .open(path.as_ref()).unwrap();

        //Copy the file into the archive and compress it
        //    Move the compressed data and get the size of the data moved
        let start = self.fp.stream_position().unwrap();
        lzma_compress(& mut std::io::BufReader::new(&fp), & mut self.fp).unwrap();
        let compressed_size = self.fp.stream_position().unwrap() - start;

        //    Make a copy of the current seek position
        let save = self.fp.stream_position().unwrap();

        //    Go back and manually add the 'compressed_size' entry to the file header
        self.fp.seek(SeekFrom::Start(position));
        bincode::serialize_into(&self.fp, &compressed_size).unwrap();

        //    Seek back to the saved position
        self.fp.seek(SeekFrom::Start(save)).unwrap();

        //Add position of the header to list
        self.version_header.insert(path.as_ref(), position);

    }

    pub fn finish(& mut self) {

        let version_header_offset = self.fp.stream_position().unwrap();

        //Append the version header
        bincode::serialize_into(&self.fp, &self.version_header).unwrap();

        //Get the size of the file (offset version directory)
        let directory_offset = self.fp.stream_len().unwrap();

        //Add the new entry in the version directory
        self.backup_directory.add(version_header_offset);

        //append the new version directory
        bincode::serialize_into(&self.fp, &self.backup_directory).unwrap();

        //set the first u64 to the offset of the version directory
        self.fp.seek(SeekFrom::Start(0)).unwrap();

        bincode::serialize_into(&self.fp, &directory_offset);
    }

}

/*impl Drop for AppendArchive {
    fn drop(&mut self) {
        self.finish();
    }
}*/

pub struct ReadArchive {
    fp: File,
    version_headers: Vec<Version>,
}

impl ReadArchive {
    fn new(archive_path: & Path) -> Self {

        let mut fp = OpenOptions::new()
            .read(true)
            .open(archive_path).unwrap();

        //Get the first u64 (version directory offset)
        let version_directory_offset = bincode::deserialize_from::<_, u64>(&fp).unwrap();

        //Seek to directory
        fp.seek(SeekFrom::Start(version_directory_offset));

        println!("version_directory_offset: {}", version_directory_offset);

        //Get directory
        let version_directory = bincode::deserialize_from::<_, VersionDirectory>(&fp).unwrap();

        println!("directory: {:?}", version_directory);

        let mut version_headers = Vec::new();


        for offset in version_directory.directory.iter() {
            let mut file_header_map = HashMap::new();

            fp.seek(SeekFrom::Start(*offset));

            let header = bincode::deserialize_from::<_, VersionHeader>(&fp).unwrap(); //bincode::deserialize::<VersionHeader>(&buffer).unwrap();

            for (file_path, file_header_offset) in header.files.iter() {
                fp.seek(SeekFrom::Start(*file_header_offset));

                let file_head = bincode::deserialize_from::<_, FileHeader>(&fp).unwrap();

                file_header_map.insert(file_path.clone(), (fp.stream_position().unwrap(), file_head));

            }

            let version = Version::new(file_header_map, header.number, header.message.clone());

            println!("Version: {:?}", version);

            version_headers.push(version);

        }

        ReadArchive {
            fp,
            version_headers,
        }
    }

    pub fn file<W: Write, P: AsRef<Path>>(& mut self, version: usize, path: P, mut writer: & mut W) -> Option<()> {


        let version = self.version_headers.get(version).unwrap();

        let (offset, header) = version.files.get(path.as_ref())?;

        let size = header.compressed_size;

        self.fp.seek(SeekFrom::Start(*offset));

        let mut taken = std::io::Read::by_ref(&mut self.fp).take(size);

        lzma_decompress(& mut std::io::BufReader::new(& mut taken), & mut writer).unwrap();

        //std::io::copy(& mut taken, & mut writer).unwrap();

        Some(())
    }
}
