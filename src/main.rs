use image::GenericImageView;
use image::{self, DynamicImage, GenericImage, ImageBuffer, Rgba, SubImage};
use mcq;
use num::Integer;
use serde_derive::{Deserialize, Serialize};
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use walkdir::{DirEntry, WalkDir};

const MMCQ_MAX_COLOR: u32 = 256;
const THUMBNAIL_SIZE: u32 = 64;
const CHUNK_SIZE: u32 = 32;
const METADATA_FILENAME: &str = "mosaic.json";

#[derive(Serialize, Deserialize, Debug)]
struct ProcessedPictureMetadata {
    pictures: Vec<ProcessedPicture>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ProcessedPicture {
    path: String,
    color_rgb: [u8; 3],
    ratio_width: u32,
    ratio_height: u32,
}

fn compute_main_color(img: &ImageBuffer<Rgba<u8>, Vec<u8>>) -> [u8; 3] {
    let mmcq = mcq::MMCQ::from_pixels_u8_rgba(img, MMCQ_MAX_COLOR);
    let palette = mmcq.get_quantized_colors();
    [palette[0].red, palette[0].grn, palette[0].blu]
}

fn compute_ratio(w: u32, h: u32) -> (u32, u32) {
    let gcd = w.gcd(&h);
    (w / gcd, h / gcd)
}

fn ratio_to_dim(ratio: (u32, u32), size: u32) -> (u32, u32) {
    let ratio_f = ratio.0 as f32 / ratio.1 as f32;
    if ratio_f > 1.0 {
        (size, (size * ratio.1 / ratio.0) as u32)
    } else {
        ((size * ratio.0 / ratio.1) as u32, size)
    }
}

fn files_from_folder(folder_path: &str) -> impl Iterator<Item = DirEntry> {
    WalkDir::new(folder_path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
}

fn process_pictures(files: &[walkdir::DirEntry], output_folder: &Path) -> Vec<ProcessedPicture> {
    let mut res = Vec::new();

    let files_nb = files.len();
    for (i, file) in files.iter().enumerate() {
        let path = file.path();
        print!("[{}/{}] {} ", i, files_nb, path.display());

        let img = match image::open(path) {
            Ok(img) => img,
            Err(_) => {
                println!("skip");
                continue;
            }
        };

        let ratio = {
            let (w, h) = img.dimensions();
            compute_ratio(w, h)
        };

        let thumb = img.thumbnail(THUMBNAIL_SIZE, THUMBNAIL_SIZE);
        let thumb_name = path.file_name().unwrap();
        let thumb_path = output_folder.join(thumb_name);
        if thumb.save(&thumb_path).is_err() {
            println!("skip");
            continue;
        }

        let processed = ProcessedPicture {
            path: thumb_name.to_string_lossy().to_string(),
            color_rgb: compute_main_color(&img.to_rgba()),
            ratio_width: ratio.0,
            ratio_height: ratio.1,
        };

        println!(
            "rgb: ({}, {}, {})",
            processed.color_rgb[0], processed.color_rgb[1], processed.color_rgb[2]
        );
        res.push(processed);
    }

    res
}

fn save_processed_pictures_metadata(
    metadata: &ProcessedPictureMetadata,
    processed_folder: &Path,
) -> Result<(), Box<Error>> {
    if !processed_folder.exists() {
        fs::create_dir(&processed_folder).unwrap();
    }

    let path = processed_folder.join(METADATA_FILENAME);
    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, metadata)?;
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

fn color_distance(c1: &[u8; 3], c2: &[u8; 3]) -> u32 {
    // http://godsnotwheregodsnot.blogspot.com/2011/09/weighted-euclidean-color-distance.html
    (((22 * (c1[0] as i32 - c2[0] as i32)).pow(2)
        + (43 * (c1[1] as i32 - c2[1] as i32)).pow(2)
        + (34 * (c1[2] as i32 - c2[2] as i32)).pow(2)) as f64)
        .sqrt() as u32
}

fn find_closest_pic_by_color<'a>(
    pics: &'a [ProcessedPicture],
    color: &[u8; 3],
) -> &'a ProcessedPicture {
    let mut closest = (&pics[0], color_distance(&pics[0].color_rgb, color));
    for i in 1..pics.len() {
        let dist = color_distance(&pics[i].color_rgb, color);
        if dist == 0 {
            return &pics[i];
        }

        if dist < closest.1 {
            closest = (&pics[i], dist);
        }
    }
    closest.0
}

fn compute_main_color_by_chunk(img: &DynamicImage, chunk_w: u32, chunk_h: u32) -> Vec<[u8; 3]> {
    let mut res = Vec::new();
    let (w, h) = img.dimensions();
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let chunk = img.view(x, y, chunk_w, chunk_h);
            res.push(compute_main_color(&chunk.to_image()));
            x += chunk_w;
        }
        y += chunk_h;
    }
    res
}

fn create_mosaic(
    model: &DynamicImage,
    processed_folder: &Path,
    pics: &[ProcessedPicture],
    ratio: (u32, u32),
) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let chunk_dim = ratio_to_dim(ratio, CHUNK_SIZE);
    let color_by_chunk = compute_main_color_by_chunk(model, chunk_dim.0, chunk_dim.1);

    let thumb_dim = ratio_to_dim(ratio, THUMBNAIL_SIZE);

    let mut res = ImageBuffer::new(
        model.width() / chunk_dim.0 * thumb_dim.0,
        model.height() / chunk_dim.1 * thumb_dim.1,
    );

    let mut x = 0;
    let mut y = 0;
    for color in color_by_chunk {
        let pic = find_closest_pic_by_color(pics, &color);
        let thumb_path = processed_folder.join(&pic.path);
        let thumb = image::open(thumb_path).unwrap();
        assert!(res.copy_from(&thumb, x, y));

        x += thumb_dim.0;
        if x >= res.width() {
            x = 0;
            y += thumb_dim.1;
        }
    }

    res
}

fn main() {
    // let files: Vec<_> = files_from_folder("/home/greg/Downloads/Takeout/Google Photos").collect();

    let processed_folder = Path::new("./processed");

    /*
    let metadata =
        ProcessedPictureMetadata { pictures: process_pictures(&files, &processed_folder) };
    save_processed_pictures_metadata(&metadata, &processed_folder).unwrap();
    */
    let metadata = load_processed_pictures_metadata(&processed_folder).unwrap();

    let model = image::open("./bruxelles.jpg").unwrap();
    let (w, h) = model.dimensions();
    let ratio = compute_ratio(w, h);
    let pics: Vec<_> = metadata
        .pictures
        .into_iter()
        .filter(|pic| pic.ratio_width == ratio.0 && pic.ratio_height == ratio.1)
        .collect();

    if pics.is_empty() {
        eprintln!("No processed pictures found with the same ratio ({}/{})", ratio.0, ratio.1);
        std::process::exit(1);
    }

    println!("{} pictures found with the same ratio ({}/{})", pics.len(), ratio.0, ratio.1);
    let mosaic = create_mosaic(&model, &processed_folder, &pics, ratio);
    mosaic.save("mosaic.jpg").unwrap();
}
