extern crate image;

use std::os;
use std::io::fs::{PathExtensions, readdir, File};
use std::io::{BufferedReader, IoError};
use std::collections::HashSet;
use std::ascii::OwnedAsciiExt;
use std::num::Float;
use std::cmp::partial_min;
use std::error::FromError;

use image::{GenericImage, ImageDecoder, ImageError};
use image::jpeg::JPEGDecoder;

static WGS84_BBOX: &'static [f64] = &[-180.0, -90.0, 180.0, 90.0];
static WGS84_WKT: &'static str = include_str!("4326.esriwkt");
static README_TEXT: &'static str = include_str!("../README");


#[derive(Show)]
enum GeoRefError {
    Io(IoError),
    Image(ImageError)
}

impl FromError<IoError> for GeoRefError {
    fn from_error(err: IoError) -> GeoRefError {
        GeoRefError::Io(err)
    }
}

impl FromError<ImageError> for GeoRefError {
    fn from_error(err: ImageError) -> GeoRefError {
        GeoRefError::Image(err)
    }
}


fn is_supported_extension(ext: Option<&str>) -> bool {
    let supported: HashSet<&str> = vec!("jpg", "png", "jpeg", "bmp", "tiff", "tif").into_iter().collect();
    match ext {
        None => false,
        Some(ext_str) => {
            let ext_string_lc = ext_str.to_string().into_ascii_lowercase();
            let ext_str_lc = ext_string_lc.as_slice();
            supported.contains(ext_str_lc)
        }
    }
}

fn difference(v1: f64, v2: f64) -> f64 {
    (v1 - v2).abs()
}

fn calculate_geotransform(width: u32, height: u32) -> [f64; 6] {

    let extent_world = [difference(WGS84_BBOX[0], WGS84_BBOX[2]),
                        difference(WGS84_BBOX[1], WGS84_BBOX[3])];
    let ratio_world = extent_world[0] / extent_world[1];
    let ratio_img = width as f64 / height as f64;

    let mut extent_img = extent_world.clone();
    if ratio_world > ratio_img {
        extent_img[0] = extent_img[0] / ratio_img;
    } else {
        extent_img[1] = extent_img[1] / ratio_img;
    }

    let center_world = [
        partial_min(WGS84_BBOX[0], WGS84_BBOX[2]).expect("no min") + ( extent_world[0] / 2.0),
        partial_min(WGS84_BBOX[1], WGS84_BBOX[3]).expect("no min") + ( extent_world[1] / 2.0)
    ];


    [
        //  pixel size in the x-direction in map units/pixel
        extent_img[0] / width as f64,

        // rotation about y-axis
        0.0f64,

        // rotation about x-axis
        0.0f64,

        // pixel size in the y-direction in map units, almost always negative
        extent_img[1] / height as f64 * -1.0f64,

        // x-coordinate of the center of the upper left pixel
        center_world[0] - (extent_img[0] / 2.0),

        // y-coordinate of the center of the upper left pixel
        center_world[1] + (extent_img[1] / 2.0)
    ]
}

fn read_image_size(imagepath: &Path) -> Result<(u32, u32), GeoRefError> {
    let reader = BufferedReader::new(File::open(imagepath));

    // optimized code path for JPEGs - attempt to read jpeg headers
    let mut jpegdecoder = JPEGDecoder::new(reader);
    match jpegdecoder.dimensions() {
        Ok(dims) => return Ok(dims), 
        Err(_) => {} // ignore
    }

    // fallback - decode the whole image
    // opening the complete images is dead-slow, especially for large ones. 
    // see https://github.com/PistonDevelopers/image/issues/99
    let img = try!(image::open(imagepath));
    Ok(img.dimensions())
}

fn pseudo_georef(imagepath: &Path) -> Result<(), GeoRefError> {
    println!("pseudo-georeferencing {}", imagepath.as_str().unwrap());

    let (width, height) = try!(read_image_size(imagepath));

    // generate world file. See: http://en.wikipedia.org/wiki/World_file
    let mut wld_file = try!(File::create(&imagepath.with_extension("wld")));
    for n in calculate_geotransform(width, height).iter() {
        try!(wld_file.write_fmt(format_args!("{}\n", n)));
    }

    // generate projection file
    let mut proj_file = try!(File::create(&imagepath.with_extension("prj")));
    try!(proj_file.write_str(WGS84_WKT));

    Ok(())
}


fn print_usage() {
    println!("Usage:\npseudo-georef [DIRECTORY] ...\n");
    println!("{}\n", README_TEXT);
}

fn main() {

    if os::args().len() < 2 {
        print_usage();
        return;
    }

    println!("Running pseudo-georef ...");

    for dir in os::args().tail().iter() {
        let path = Path::new(dir);
        if !path.is_dir() {
            panic!("Path {} is not a directory", dir);
        }
        let entites = match readdir(&path) {
            Ok(p) => p,
            Err(_) => {
                panic!("Could not read directory {}", dir)
            }
        };

        for entity in entites.iter().filter(|&x| is_supported_extension(x.extension_str())) {
            match pseudo_georef(entity) {
                Ok(()) => {},
                Err(e) => {
                    panic!("{}", e);
                }
            }
        }
    }
}
