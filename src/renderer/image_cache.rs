use std::collections::HashMap;
use base64::{
    Engine as _,
    engine::general_purpose::STANDARD_NO_PAD,
};
use skia_safe::{Image, Data};

pub struct ImageCache {
    images: HashMap<u64, Image>
}

impl ImageCache {
    pub fn new() -> Self {
        Self {
            images: HashMap::new()
        }
    }

    pub fn upload_image(&mut self, id: u64, data: &String) {
        log::info!("upload image");
        let image_data = STANDARD_NO_PAD.decode(data).unwrap();
        // TODO: Don't copy
        let image_data = Data::new_copy(&image_data);
        let image = Image::from_encoded(image_data).unwrap();
        log::info!("Image loaded {:?}", image);
        self.images.insert(id, image);
    }
}
