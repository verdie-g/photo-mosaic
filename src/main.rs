use image::GenericImageView;
use image::{self, DynamicImage};
use mcq;
use num::Integer;
use serde_derive::{Deserialize, Serialize};
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

const MMCQ_MAX_COLOR: u32 = 256;
const THUMBNAIL_SIZE: u32 = 64;
const METADATA_FILENAME: &str = "mosaic.json";

#[derive(Serialize, Deserialize, Debug)]
struct ProcessedPictureMetadata {
    pictures: Vec<ProcessedPicture>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ProcessedPicture {
    path: String,
    color: u32,
    ratio_width: u32,
    ratio_height: u32,
}

fn compute_main_color(img: &DynamicImage) -> u32 {
    let rgba_pixels = img.to_rgba().into_raw();
    let mmcq = mcq::MMCQ::from_pixels_u8_rgba(&rgba_pixels, MMCQ_MAX_COLOR);
    let palette = mmcq.get_quantized_colors();
    palette[0].rgb
}

fn compute_ratio(w: u32, h: u32) -> (u32, u32) {
    let gcd = w.gcd(&h);
    (w / gcd, h / gcd)
}

fn files_from_folder(folder_path: &str) -> impl Iterator<Item = DirEntry> {
    WalkDir::new(folder_path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
}

fn process_pictures(files: &Vec<walkdir::DirEntry>, output_folder: &Path) -> Vec<ProcessedPicture> {
    let mut res = Vec::new();

    let files_nb = files.len();
    for (i, file) in files.into_iter().enumerate() {
        let path = file.path();
        println!("[{}/{}] {}", i, files_nb, path.display());

        let img = match image::open(path) {
            Ok(img) => img,
            Err(_) => continue,
        };

        let ratio = {
            let (w, h) = img.dimensions();
            compute_ratio(w, h)
        };

        let thumb = img.thumbnail(THUMBNAIL_SIZE, THUMBNAIL_SIZE);
        let thumb_name = path.file_name().unwrap();
        let thumb_path = output_folder.join(thumb_name);
        thumb.save(&thumb_path).unwrap();

        let processed = ProcessedPicture {
            path: thumb_name.to_string_lossy().to_string(),
            color: compute_main_color(&img),
            ratio_width: ratio.0,
            ratio_height: ratio.1,
        };
        res.push(processed);
    }

    res
}

fn save_processed_pictures_metadata(
    pics: &[ProcessedPicture],
    processed_folder: &Path,
) -> Result<(), Box<Error>> {
    if !processed_folder.exists() {
        fs::create_dir(&processed_folder).unwrap();
    }

    let path = processed_folder.join(METADATA_FILENAME);
    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, pics)?;
    Ok(())
}

fn load_processed_pictures_metadata(
    processed_folder: &Path,
) -> Result<ProcessedPictureMetadata, Box<Error>> {
    let path = processed_folder.join(METADATA_FILENAME);
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let metadata: ProcessedPictureMetadata = serde_json::from_reader(reader)?;
    Ok(metadata)
}

fn main() {
    // let files: Vec<_> = files_from_folder("/home/greg/Downloads/instagram/stories").collect();

    let processed_folder = Path::new("./processed");

    // let pics = process_pictures(&files, &processed_folder);
    // save_processed_pictures_metadata(&pics, &processed_folder).unwrap();
    let pics = load_processed_pictures_metadata(&processed_folder);
}
