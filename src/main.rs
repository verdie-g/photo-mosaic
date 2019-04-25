use clap::{App, Arg, SubCommand};
use image::GenericImageView;
use image::{self, imageops, DynamicImage, GenericImage, ImageBuffer, Rgba, SubImage};
use num::Integer;
use serde_derive::{Deserialize, Serialize};
use std::cmp;
use std::error::Error;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::Path;
use walkdir::{DirEntry, WalkDir};

const CONTRAST_ADJUSTMENT: f32 = 20.0;
const THUMBNAIL_SIZE: u32 = 64;
const CHUNK_SIZE: u32 = 8;
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
    let mut color_sums: [u32; 3] = [0; 3];
    for pixel in img.enumerate_pixels() {
        for i in 0..3 {
            color_sums[i] += u32::from(pixel.2[i]);
        }
    }

    let mut avg_color = [0; 3];
    for i in 0..3 {
        avg_color[i] = (color_sums[i] / (img.width() * img.height())) as u8;
    }
    avg_color
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

fn files_from_folder(folder_path: &Path) -> impl Iterator<Item = DirEntry> {
    WalkDir::new(folder_path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
}

fn image_square_view(img: &DynamicImage) -> SubImage<&DynamicImage> {
    let (w, h) = img.dimensions();
    let square_size = cmp::min(w, h);

    let x_offset = (w - square_size) / 2;
    let y_offset = (h - square_size) / 2;
    img.view(x_offset, y_offset, square_size, square_size)
}

fn process_pictures(files: &[walkdir::DirEntry], output_folder: &Path) -> Vec<ProcessedPicture> {
    if !output_folder.exists() {
        fs::create_dir(&output_folder).unwrap();
    }

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

        let square = image_square_view(&img);
        let thumb = imageops::thumbnail(&square, THUMBNAIL_SIZE, THUMBNAIL_SIZE);
        let thumb = imageops::contrast(&thumb, CONTRAST_ADJUSTMENT);
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

fn color_distance(c1: [u8; 3], c2: [u8; 3]) -> u32 {
    let mut a = 0;
    for i in 0..3 {
        a += (i32::from(c1[i]) - i32::from(c2[i])).pow(2);
    }
    f64::from(a).sqrt() as u32
}

fn find_closest_pic_by_color(pics: &[ProcessedPicture], color: [u8; 3]) -> &ProcessedPicture {
    let mut closest = (&pics[0], color_distance(pics[0].color_rgb, color));
    for i in 1..pics.len() {
        let dist = color_distance(pics[i].color_rgb, color);
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
    while y + chunk_h <= h {
        let mut x = 0;
        while x + chunk_w <= w {
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
        let pic = find_closest_pic_by_color(pics, color);
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

fn cmd_preprocess(gallery_folder: &Path, output_folder: &Path) {
    let files: Vec<_> = files_from_folder(gallery_folder).collect();
    let metadata = ProcessedPictureMetadata { pictures: process_pictures(&files, output_folder) };
    save_processed_pictures_metadata(&metadata, output_folder).unwrap();
}

fn cmd_create(preprocessed_folder: &Path, model: &Path, output_image: &Path) {
    let metadata = load_processed_pictures_metadata(preprocessed_folder).unwrap();

    let model = image::open(model).unwrap();
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
    let mosaic = create_mosaic(&model, preprocessed_folder, &pics, ratio);
    mosaic.save(output_image).unwrap();
}

fn main() {
    let matches = App::new("Photo Mosaic")
        .version("0.1")
        .author("verdie-g <gregoire.verdier@gmail.com>")
        .about("Create a photo mosaic")
        .subcommands(vec![
            SubCommand::with_name("preprocess")
                .about("Recursively traverses your gallery to preprocess all image files")
                .arg(
                    Arg::with_name("gallery_folder")
                        .help("Sets the path of your gallery")
                        .index(1)
                        .required(true),
                )
                .arg(
                    Arg::with_name("output_folder")
                        .help("Sets the path of the output folder for the processed images")
                        .index(2)
                        .required(true),
                ),
            SubCommand::with_name("create")
                .about("Create a photo mosaic from a preprocessed gallery and a model image")
                .arg(
                    Arg::with_name("preprocessed_folder")
                        .help("Sets the path of the folder with the preprocessed pictures")
                        .index(1)
                        .required(true),
                )
                .arg(
                    Arg::with_name("model")
                        .help("Sets the path of image model")
                        .index(2)
                        .required(true),
                )
                .arg(
                    Arg::with_name("output_image")
                        .help("Sets the output path of the created mosaic")
                        .index(3)
                        .required(true),
                ),
        ])
        .get_matches();

    match matches.subcommand() {
        ("preprocess", Some(cmd_matches)) => {
            let gallery_folder = Path::new(cmd_matches.value_of("gallery_folder").unwrap());
            let output_folder = Path::new(cmd_matches.value_of("output_folder").unwrap());
            cmd_preprocess(gallery_folder, output_folder);
        }
        ("create", Some(cmd_matches)) => {
            let preprocessed_folder =
                Path::new(cmd_matches.value_of("preprocessed_folder").unwrap());
            let model = Path::new(cmd_matches.value_of("model").unwrap());
            let output_image = Path::new(cmd_matches.value_of("output_image").unwrap());
            cmd_create(preprocessed_folder, model, output_image);
        }
        _ => panic!(),
    }
}
