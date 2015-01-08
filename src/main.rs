#![feature(macro_rules)]

extern crate image;

use std::os;
use std::io::fs::{PathExtensions, readdir, File};
use std::io::{BufferedReader, IoError};
use std::ascii::OwnedAsciiExt;
use std::num::Float;
use std::cmp::partial_min;
use std::error::FromError;
use std::iter::IteratorExt;

use image::{GenericImage, ImageDecoder, ImageError};
use image::jpeg::JPEGDecoder;


static CRS_BBOX: &'static [f64] = &[-20026376.39, -20048966.10, 20026376.39, 20048966.10];
static CRS_WKT: &'static str = include_str!("3857.esriwkt");
static README_TEXT: &'static str = include_str!("../README");
static SUPPORTED_FORMAT_EXTS: &'static [&'static str] = &[
    "jpg", 
    "jpeg", 
    "png", 
    "gif", 
    "tiff", 
    "tif"
];


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


/// absolute difference between two values
macro_rules! difference {
    ($a:expr, $b:expr) => { ($a - $b).abs() }
}

struct BoundingBox {
    minx: f64,
    miny: f64,
    maxx: f64,
    maxy: f64
}

impl BoundingBox {
    fn width(&self) -> f64 {
        difference!(self.minx, self.maxx)
    }

    fn height(&self) -> f64 {
        difference!(self.miny, self.maxy)
    }
}

struct RasterSize {
    width: u32,
    height: u32
}

struct RefBox {
    bbox: BoundingBox,
    size: RasterSize
}


impl RefBox {

    fn new(width: u32, height: u32) -> RefBox {
        let raster_size = RasterSize { width: width, height: height };
        
        let extent_world = [difference!(CRS_BBOX[0], CRS_BBOX[2]),
                            difference!(CRS_BBOX[1], CRS_BBOX[3])];
        let ratio_world = extent_world[0] / extent_world[1];
        let ratio_img = raster_size.width as f64 / raster_size.height as f64;

        let mut extent_img = extent_world.clone();
        if ratio_world > ratio_img {
            extent_img[0] = extent_img[0] / ratio_img;
        } else {
            extent_img[1] = extent_img[1] / ratio_img;
        }

        let center_world = [
            partial_min(CRS_BBOX[0], CRS_BBOX[2]).expect("no min")
                    + ( extent_world[0] / 2.0),
            partial_min(CRS_BBOX[1], CRS_BBOX[3]).expect("no min")
                    + ( extent_world[1] / 2.0)
        ];

        RefBox {
            size: raster_size,
            bbox: BoundingBox {
                minx: center_world[0] - (extent_img[0] / 2.0),
                miny: center_world[1] - (extent_img[1] / 2.0),
                maxx: center_world[0] + (extent_img[0] / 2.0),
                maxy: center_world[1] + (extent_img[1] / 2.0)
            }
        }
    }

    fn world_file_values(&self) -> [f64; 6] {
        [
            //  pixel size in the x-direction in map units/pixel
            self.bbox.width() / self.size.width as f64,

            // rotation about y-axis
            0.0f64,

            // rotation about x-axis
            0.0f64,

            // pixel size in the y-direction in map units, almost always negative
            self.bbox.height() / self.size.height as f64 * -1.0f64,

            // x-coordinate of the center of the upper left pixel
            self.bbox.minx,

            // y-coordinate of the center of the upper left pixel
            self.bbox.maxy
        ]
    }

}


fn is_supported_extension(ext: Option<&str>) -> bool {
    match ext {
        None => false,
        Some(ext_str) => {
            let ext_string_lc = ext_str.to_string().into_ascii_lowercase();
            let ext_str_lc = ext_string_lc.as_slice();

            SUPPORTED_FORMAT_EXTS.iter()
                .find(|&x| { *x == ext_str_lc })
                .is_some()
        }
    }
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
    let refbox = RefBox::new(width, height);

    // generate world file. See: http://en.wikipedia.org/wiki/World_file
    let mut wld_file = try!(File::create(&imagepath.with_extension("wld")));
    for n in refbox.world_file_values().iter() {
        try!(wld_file.write_fmt(format_args!("{}\n", n)));
    }

    // generate projection file
    let mut proj_file = try!(File::create(&imagepath.with_extension("prj")));
    try!(proj_file.write_str(CRS_WKT));

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
