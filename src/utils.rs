use std::{sync::{Arc, Mutex}, path::Path};

use log::{error, debug};
use openslide::OpenSlide;
use tch::{Tensor, Device, Kind, index::*};
use tch_utils::image::ImageTensorExt;

use crate::geojson;

pub type CratePoint = [f32; 2];
pub type TchUtilsPoint = (f64, f64);
pub type Points = Vec<CratePoint>;

pub trait CratePointExt {
    fn to_tchutils_point(&self) -> TchUtilsPoint;
    fn into_tchutils_point(self) -> TchUtilsPoint;
}

impl CratePointExt for CratePoint {
    fn to_tchutils_point(&self) -> TchUtilsPoint {
        (self[0] as f64, self[1] as f64)
    }

    fn into_tchutils_point(self) -> TchUtilsPoint {
        (self[0] as f64, self[1] as f64)
    }
}

pub trait PointsExt {
    fn to_tchutils_points(&self) -> Vec<TchUtilsPoint>;
    fn into_tchutils_points(self) -> Vec<TchUtilsPoint>;
}

impl PointsExt for Points {
    fn to_tchutils_points(&self) -> Vec<TchUtilsPoint> {
        self.iter().map(|point| point.to_tchutils_point()).collect()
    }

    fn into_tchutils_points(self) -> Vec<TchUtilsPoint> {
        self.into_iter()
            .map(|point| point.into_tchutils_point())
            .collect()
    }
}

/**
Preprocess the geojson polygon to extract the centroid and the centered points of the polygon
 */
pub(crate) fn preprocess_polygon(feature: &geojson::Feature) -> ([f32; 2], Vec<[f32; 2]>) {
    let polygone = &feature.geometry.coordinates[0];
    let centroid = polygone.iter().fold([0.0, 0.0], |mut acc, point| {
        acc[0] += point[0];
        acc[1] += point[1];
        acc
    });
    let centroid = [
        centroid[0] / polygone.len() as f32,
        centroid[1] / polygone.len() as f32,
    ];
    let centered_polygone = polygone
        .iter()
        .map(|point| {
            let x = point[0] - centroid[0];
            let y = point[1] - centroid[1];
            [x, y]
        })
        .collect::<Vec<_>>();
    (centroid, centered_polygone)
}

/**
Takes a chunk of geojson features and returns a tuple of tensors containing the patches and the masks
 */
pub(crate) fn load_slides(
    features:&[geojson::Feature], 
    slide: Arc<Mutex<OpenSlide>>, 
    patch_size: usize
    ) -> (Vec<[f32;2]>, Vec<bool>, Tensor, Tensor){
    let (centroid_err, patch_mask) : (Vec<_>, Vec<_>) = features.into_iter()
        .map(preprocess_polygon)
        .map(|(centroid, centered_polygone)|{
            let mask = tch_utils::shapes::polygon(patch_size, patch_size, &centered_polygone.to_tchutils_points(), (Kind::Float, Device::Cpu));
            let patch = {
                let slide = slide.lock().unwrap();
                let left = (centroid[0] - patch_size as f32 / 2.0) as usize;
                let top = (centroid[1] - patch_size as f32 / 2.0) as usize;
                let start = std::time::Instant::now();
                let res = slide.read_region(top, left, 0, patch_size, patch_size);
                debug!("read_region took {:?}", start.elapsed());
                res
            };
            match patch {
                Ok(ok) => {
                    let tensor = Tensor::from_image(ok.into()).i(..3);
                    if tensor.size().as_slice() != &[3, patch_size as i64, patch_size as i64] {
                        let padded = Tensor::zeros(&[3, patch_size as i64, patch_size as i64], (Kind::Float, Device::Cpu));
                        padded.i((..tensor.size()[0], ..tensor.size()[1], ..tensor.size()[2])).copy_(&tensor);
                        ((centroid, false), (padded, mask))
                    } else {
                        ((centroid, false), (tensor, mask))
                    }
                },
                Err(err) => {
                    error!("Error while reading patch for nuclei {centroid:?} : {}", err);
                    ((centroid, true), (Tensor::ones(&[3, patch_size as i64, patch_size as i64], (Kind::Float, Device::Cpu)), mask))
                },
            }
        })
        .unzip();
    let (centroids, err): (Vec<_>, Vec<_>) = centroid_err.into_iter().unzip();
    let (patches, masks):(Vec<_>, Vec<_>) = patch_mask.into_iter().unzip();
    let patches = Tensor::stack(&patches, 0);
    let masks = Tensor::stack(&masks, 0);
    debug!("patches size {:?}", patches.size());
    (centroids, err, patches, masks)
}

/**
Takes a chunk of geojson features and returns a tuple of tensors containing the patches and the masks
 */
pub(crate) fn load_slides2(
    features:&[geojson::Feature], 
    slide: &Path, 
    patch_size: usize
    ) -> (Vec<[f32;2]>, Vec<bool>, Tensor, Tensor){
    let slide = OpenSlide::new(slide).unwrap();
    let (centroid_err, patch_mask) : (Vec<_>, Vec<_>) = features.into_iter()
        .map(preprocess_polygon)
        .map(|(centroid, centered_polygone)|{
            let mask = tch_utils::shapes::polygon(patch_size, patch_size, &centered_polygone.to_tchutils_points(), (Kind::Float, Device::Cpu));
            let patch = {
                let left = (centroid[0] - patch_size as f32 / 2.0) as usize;
                let top = (centroid[1] - patch_size as f32 / 2.0) as usize;
                let res = slide.read_region(top, left, 0, patch_size, patch_size);
                res
            };
            match patch {
                Ok(ok) => {
                    let tensor = Tensor::from_image(ok.into()).i(..3);
                    if tensor.size().as_slice() != &[3, patch_size as i64, patch_size as i64] {
                        let padded = Tensor::zeros(&[3, patch_size as i64, patch_size as i64], (Kind::Float, Device::Cpu));
                        padded.i((..tensor.size()[0], ..tensor.size()[1], ..tensor.size()[2])).copy_(&tensor);
                        ((centroid, false), (padded, mask))
                    } else {
                        ((centroid, false), (tensor, mask))
                    }
                },
                Err(err) => {
                    error!("Error while reading patch for nuclei {centroid:?} : {}", err);
                    ((centroid, true), (Tensor::ones(&[3, patch_size as i64, patch_size as i64], (Kind::Float, Device::Cpu)), mask))
                },
            }
        })
        .unzip();
    let (centroids, err): (Vec<_>, Vec<_>) = centroid_err.into_iter().unzip();
    let (patches, masks):(Vec<_>, Vec<_>) = patch_mask.into_iter().unzip();
    let patches = Tensor::stack(&patches, 0);
    let masks = Tensor::stack(&masks, 0);
    debug!("patches size {:?}", patches.size());
    (centroids, err, patches, masks)
}


pub(crate) fn move_tensors_to_device((centroid, err, mut patches, mut masks):(Vec<[f32;2]>, Vec<bool>, Tensor, Tensor), gpus: Option<Vec<usize>>) -> (Vec<[f32;2]>, Vec<bool>, Tensor, Tensor){
    if let Some (gpus) = &gpus {
        let gpus = gpus.clone();
        let gpu_count = gpus.len();
        let gpu_idx = rayon::current_thread_index().unwrap() % gpu_count;
        patches = patches.to_device(Device::Cuda(gpus[gpu_idx]));
        masks = masks.to_device(Device::Cuda(gpus[gpu_idx]));
    }
    
    (centroid, err, patches, masks)
}