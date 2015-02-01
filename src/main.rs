#![feature(core)]
#![feature(os)]
#![feature(io)]
#![feature(collections)]
#![feature(path)]
#![feature(std_misc)]

extern crate image;
extern crate getopts;
extern crate "rustc-serialize" as rustc_serialize;

use std::old_io::fs::{PathExtensions, readdir, File};
use std::old_io::{BufferedReader, IoError};
use std::ascii::OwnedAsciiExt;
use std::num::Float;
use std::cmp::partial_min;
use std::error::FromError;
use std::iter::IteratorExt;
use std::string::FromUtf8Error;

use rustc_serialize::json;

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


#[derive(Debug)]
enum GeoRefError {
    Io(IoError),
    Image(ImageError),
    FromUtf8Error(FromUtf8Error)
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

impl FromError<FromUtf8Error> for GeoRefError {
    fn from_error(err: FromUtf8Error) -> GeoRefError {
        GeoRefError::FromUtf8Error(err)
    }
}


/// absolute difference between two values
macro_rules! difference {
    ($a:expr, $b:expr) => { ($a - $b).abs() }
}

#[derive(RustcEncodable)]
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

#[derive(RustcEncodable)]
struct RasterSize {
    width: u32,
    height: u32
}

#[derive(RustcEncodable)]
struct RefBox {
    bbox: BoundingBox,
    size: RasterSize,
    name: Option<String>,
    filename: Option<String>
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
            },
            name: None,
            filename: None
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

fn pseudo_georef(imagepath: &Path) -> Result<RefBox, GeoRefError> {
    println!("pseudo-georeferencing {}", imagepath.as_str().unwrap_or("?"));

    let (width, height) = try!(read_image_size(imagepath));
    let mut refbox = RefBox::new(width, height);

    let stem_res = imagepath.filestem_str();
    if stem_res.is_some() {
        refbox.name = Some(String::from_str(stem_res.unwrap()));
    }
    refbox.filename = Some(try!(String::from_utf8(imagepath.clone().into_vec())));

    // generate world file. See: http://en.wikipedia.org/wiki/World_file
    let mut wld_file = try!(File::create(&imagepath.with_extension("wld")));
    for n in refbox.world_file_values().iter() {
        try!(wld_file.write_fmt(format_args!("{}\n", n)));
    }

    // generate projection file
    let mut proj_file = try!(File::create(&imagepath.with_extension("prj")));
    try!(proj_file.write_str(CRS_WKT));

    Ok(refbox)
}


fn print_usage(progname: &str, opts: getopts::Options) {
    let brief = format!("Usage:\n{} [options] DIRECTORY ...", progname);
    print!("{}\n{}\n", opts.usage(brief.as_slice()), README_TEXT);
}

fn main() {
    let args = std::os::args();
    let progname = args[0].as_slice();

    let mut opts = getopts::Options::new();
    opts.optopt("j", "json", "Write a JSON file with boundingboxes and sizes of the images", "JSON");
    opts.optflag("h", "help", "Print this help");
    let optmatches = match opts.parse(args.tail()) {
        Ok(m)   => m,
        Err(e)  => { panic!(e.to_string()) }
    };

    if optmatches.opt_present("h") {
        print_usage(progname, opts);
        return;
    }
    if optmatches.free.is_empty() {
        print_usage(progname, opts);
        std::os::set_exit_status(1);
        return;
    }

    println!("Running {} ...", progname);

    for dir in optmatches.free.iter() {
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

        let mut refboxes: Vec<RefBox> = vec!();
        for entity in entites.iter().filter(|&x| is_supported_extension(x.extension_str())) {
            match pseudo_georef(entity) {
                Ok(refbox) => {
                    refboxes.push(refbox);
                },
                Err(e) => {
                    panic!("{:?}", e);
                }
            }
        }

        if optmatches.opt_present("j") {
            let json_path = Path::new(
                    optmatches.opt_str("j").expect("Missing path of JSON file.")
                );
            let mut json_file = File::create(&json_path).unwrap();
            let json_data = match json::encode(&refboxes) {
                Ok(s) => s,
                Err(e) => {
                    panic!("{:?}", e);
                }
            };
            let jw_res = json_file.write_str(json_data.as_slice());
            if jw_res.is_err() {
                panic!("Could not write to json file: {:?}", jw_res.err());
            };

        }
    }
}
