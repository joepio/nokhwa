/*
 * Copyright 2022 l1npengtul <l1npengtul@protonmail.com> / The Nokhwa Contributors
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use crate::Camera;
use image::{ImageBuffer, Rgb};
use nokhwa_core::{
    buffer::Buffer,
    error::NokhwaError,
    traits::CaptureBackendTrait,
    types::{
        ApiBackend, CameraControl, CameraFormat, CameraIndex, CameraInfo, ControlValueSetter,
        FrameFormat, KnownCameraControl, RequestedFormat, Resolution,
    },
};
use std::{
    any::Any,
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

type AtomicLock<T> = Arc<Mutex<T>>;
pub type CallbackFn = fn(
    _camera: &Arc<Mutex<Camera>>,
    _frame_callback: &Arc<Mutex<Option<Box<dyn FnMut(Buffer) + Send + 'static>>>>,
    _last_frame_captured: &Arc<Mutex<Buffer>>,
    _die_bool: &Arc<AtomicBool>,
);
type HeldCallbackType = Arc<Mutex<Box<dyn FnMut(Buffer) + Send + 'static>>>;

/// Creates a camera that runs in a different thread that you can use a callback to access the frames of.
/// It uses a `Arc` and a `Mutex` to ensure that this feels like a normal camera, but callback based.
/// See [`Camera`] for more details on the camera itself.
///
/// Your function is called every time there is a new frame. In order to avoid frame loss, it should
/// complete before a new frame is available. If you need to do heavy image processing, it may be
/// beneficial to directly pipe the data to a new thread to process it there.
///
/// Note that this does not have `WGPU` capabilities. However, it should be easy to implement.
/// # SAFETY
/// The `Mutex` guarantees exclusive access to the underlying camera struct. They should be safe to
/// impl `Send` on.
#[cfg_attr(feature = "docs-features", doc(cfg(feature = "output-threaded")))]
pub struct CallbackCamera {
    camera: AtomicLock<Camera>,
    frame_callback: HeldCallbackType,
    last_frame_captured: AtomicLock<Buffer>,
    die_bool: Arc<AtomicBool>,
}

impl CallbackCamera {
    /// Create a new `ThreadedCamera` from an `index` and `format`. `format` can be `None`.
    /// # Errors
    /// This will error if you either have a bad platform configuration (e.g. `input-v4l` but not on linux) or the backend cannot create the camera (e.g. permission denied).
    pub fn new(
        index: CameraIndex,
        format: RequestedFormat,
        callback: impl FnMut(Buffer) + Send + 'static,
    ) -> Result<Self, NokhwaError> {
        let arc_camera = Arc::new(Mutex::new(Camera::new(index, format)?));
        Ok(CallbackCamera {
            camera: arc_camera,
            frame_callback: Arc::new(Mutex::new(Box::new(callback))),
            last_frame_captured: Arc::new(Mutex::new(Buffer::new_with_vec(
                Resolution::new(0, 0),
                &vec![],
                FrameFormat::GRAY,
            ))),
            die_bool: Arc::new(Default::default()),
        })
    }

    /// Gets the current Camera's index.
    #[must_use]
    pub fn index(&self) -> usize {
        self.camera.lock().index().clone()
    }

    /// Sets the current Camera's index. Note that this re-initializes the camera.
    /// # Errors
    /// The Backend may fail to initialize.
    pub fn set_index(&mut self, new_idx: usize) -> Result<(), NokhwaError> {
        self.camera.lock().set_index(new_idx)
    }

    /// Gets the current Camera's backend
    #[must_use]
    pub fn backend(&self) -> ApiBackend {
        self.camera.lock().backend()
    }

    /// Sets the current Camera's backend. Note that this re-initializes the camera.
    /// # Errors
    /// The new backend may not exist or may fail to initialize the new camera.
    pub fn set_backend(&mut self, new_backend: ApiBackend) -> Result<(), NokhwaError> {
        self.camera.lock().set_backend(new_backend)
    }

    /// Gets the camera information such as Name and Index as a [`CameraInfo`].
    #[must_use]
    pub fn info(&self) -> CameraInfo {
        self.camera.lock().info().clone()
    }

    /// Gets the current [`CameraFormat`].
    pub fn camera_format(&self) -> Result<CameraFormat, NokhwaError> {
        self.camera.lock().camera_format()
    }

    /// Will set the current [`CameraFormat`]
    /// This will reset the current stream if used while stream is opened.
    /// # Errors
    /// If you started the stream and the camera rejects the new camera format, this will return an error.
    pub fn set_camera_format(&mut self, new_fmt: CameraFormat) -> Result<(), NokhwaError> {
        *self.last_frame_captured.lock() =
            Buffer::new(new_res, &Vec::default(), self.camera_format()?.format());
        self.camera.lock().set_camera_format(new_fmt)
    }

    /// A hashmap of [`Resolution`]s mapped to framerates
    /// # Errors
    /// This will error if the camera is not queryable or a query operation has failed. Some backends will error this out as a [`UnsupportedOperationError`](crate::NokhwaError::UnsupportedOperationError).
    pub fn compatible_list_by_resolution(
        &mut self,
        fourcc: FrameFormat,
    ) -> Result<HashMap<Resolution, Vec<u32>>, NokhwaError> {
        self.camera.lock().compatible_list_by_resolution(fourcc)
    }

    /// A Vector of compatible [`FrameFormat`]s.
    /// # Errors
    /// This will error if the camera is not queryable or a query operation has failed. Some backends will error this out as a [`UnsupportedOperationError`](crate::NokhwaError::UnsupportedOperationError).
    pub fn compatible_fourcc(&mut self) -> Result<Vec<FrameFormat>, NokhwaError> {
        self.camera.lock().compatible_fourcc()
    }

    /// Gets the current camera resolution (See: [`Resolution`], [`CameraFormat`]).
    pub fn resolution(&self) -> Result<Resolution, NokhwaError> {
        Ok(self
            .camera
            .lock()
            .map_err(|why| NokhwaError::GetPropertyError {
                property: "Resolution".to_string(),
                error: why.to_string(),
            })?
            .resolution())
    }

    /// Will set the current [`Resolution`]
    /// This will reset the current stream if used while stream is opened.
    /// # Errors
    /// If you started the stream and the camera rejects the new resolution, this will return an error.
    pub fn set_resolution(&mut self, new_res: Resolution) -> Result<(), NokhwaError> {
        *self.last_frame_captured.lock() =
            Buffer::new_with_vec(new_res, Vec::default(), self.camera_format()?.format());
        self.camera
            .lock()
            .map_err(|why| NokhwaError::SetPropertyError {
                property: "Resolution".to_string(),
                value: new_res.to_string(),
                error: why.to_string(),
            })?
            .set_resolution(new_res)
    }

    /// Gets the current camera framerate (See: [`CameraFormat`]).
    pub fn frame_rate(&self) -> Result<u32, NokhwaError> {
        Ok(self
            .camera
            .lock()
            .map_err(|why| NokhwaError::GetPropertyError {
                property: "Framerate".to_string(),
                error: why.to_string(),
            })?
            .frame_rate())
    }

    /// Will set the current framerate
    /// This will reset the current stream if used while stream is opened.
    /// # Errors
    /// If you started the stream and the camera rejects the new framerate, this will return an error.
    pub fn set_frame_rate(&mut self, new_fps: u32) -> Result<(), NokhwaError> {
        self.camera
            .lock()
            .map_err(|why| NokhwaError::SetPropertyError {
                property: "Framerate".to_string(),
                value: new_fps.to_string(),
                error: why.to_string(),
            })?
            .set_frame_rate(new_fps)
    }

    /// Gets the current camera's frame format (See: [`FrameFormat`], [`CameraFormat`]).
    pub fn frame_format(&self) -> Result<FrameFormat, NokhwaError> {
        Ok(self
            .camera
            .lock()
            .map_err(|why| NokhwaError::GetPropertyError {
                property: "Frameformat".to_string(),
                error: why.to_string(),
            })?
            .frame_format())
    }

    /// Will set the current [`FrameFormat`]
    /// This will reset the current stream if used while stream is opened.
    /// # Errors
    /// If you started the stream and the camera rejects the new frame format, this will return an error.
    pub fn set_frame_format(&mut self, fourcc: FrameFormat) -> Result<(), NokhwaError> {
        self.camera
            .lock()
            .map_err(|why| NokhwaError::SetPropertyError {
                property: "Framerate".to_string(),
                value: fourcc.to_string(),
                error: why.to_string(),
            })?
            .set_frame_format(fourcc)
    }

    /// Gets the current supported list of [`KnownCameraControl`]
    /// # Errors
    /// If the list cannot be collected, this will error. This can be treated as a "nothing supported".
    pub fn supported_camera_controls(&self) -> Result<Vec<KnownCameraControl>, NokhwaError> {
        self.camera
            .lock()
            .map_err(|why| NokhwaError::GetPropertyError {
                property: "Supported Camera Controls".to_string(),
                error: why.to_string(),
            })
            .supported_camera_controls()
    }

    /// Gets the current supported list of [`CameraControl`]s keyed by its name as a `String`.
    /// # Errors
    /// If the list cannot be collected, this will error. This can be treated as a "nothing supported".
    pub fn camera_controls(&self) -> Result<Vec<CameraControl>, NokhwaError> {
        let known_controls = self.supported_camera_controls()?;
        let maybe_camera_controls = known_controls
            .iter()
            .map(|x| self.camera_control(*x))
            .filter(Result::is_ok)
            .map(Result::unwrap)
            .collect::<Vec<CameraControl>>();

        Ok(maybe_camera_controls)
    }

    /// Gets the current supported list of [`CameraControl`]s keyed by its name as a `String`.
    /// # Errors
    /// If the list cannot be collected, this will error. This can be treated as a "nothing supported".
    pub fn camera_controls_string(&self) -> Result<HashMap<String, CameraControl>, NokhwaError> {
        let known_controls = self.supported_camera_controls()?;
        let maybe_camera_controls = known_controls
            .iter()
            .map(|x| (x.to_string(), self.camera_control(*x)))
            .filter(|(_, x)| x.is_ok())
            .map(|(c, x)| (c, Result::unwrap(x)))
            .collect::<Vec<(String, CameraControl)>>();
        let mut control_map = HashMap::with_capacity(maybe_camera_controls.len());

        for (kc, cc) in maybe_camera_controls {
            control_map.insert(kc, cc);
        }

        Ok(control_map)
    }

    /// Gets the current supported list of [`CameraControl`]s keyed by its name as a `String`.
    /// # Errors
    /// If the list cannot be collected, this will error. This can be treated as a "nothing supported".
    pub fn camera_controls_known_camera_controls(
        &self,
    ) -> Result<HashMap<KnownCameraControl, CameraControl>, NokhwaError> {
        let known_controls = self.supported_camera_controls()?;
        let maybe_camera_controls = known_controls
            .iter()
            .map(|x| (*x, self.camera_control(*x)))
            .filter(|(_, x)| x.is_ok())
            .map(|(c, x)| (c, Result::unwrap(x)))
            .collect::<Vec<(KnownCameraControl, CameraControl)>>();
        let mut control_map = HashMap::with_capacity(maybe_camera_controls.len());

        for (kc, cc) in maybe_camera_controls {
            control_map.insert(kc, cc);
        }

        Ok(control_map)
    }

    /// Gets the value of [`KnownCameraControl`].
    /// # Errors
    /// If the `control` is not supported or there is an error while getting the camera control values (e.g. unexpected value, too high, etc)
    /// this will error.
    pub fn camera_control(
        &self,
        control: KnownCameraControl,
    ) -> Result<CameraControl, NokhwaError> {
        self.camera
            .lock()
            .map_err(|why| NokhwaError::GetPropertyError {
                property: "Camera Control".to_string(),
                error: why.to_string(),
            })?
            .camera_control(control)
    }

    /// Sets the control to `control` in the camera.
    /// Usually, the pipeline is calling [`camera_control()`](crate::CaptureBackendTrait::camera_control()), getting a camera control that way
    /// then calling one of the methods to set the value: [`set_value()`](CameraControl::set_value()) or [`with_value()`](CameraControl::with_value()).
    /// # Errors
    /// If the `control` is not supported, the value is invalid (less than min, greater than max, not in step), or there was an error setting the control,
    /// this will error.
    pub fn set_camera_control(
        &mut self,
        id: KnownCameraControl,
        control: ControlValueSetter,
    ) -> Result<(), NokhwaError> {
        self.camera
            .lock()
            .map_err(|why| NokhwaError::SetPropertyError {
                property: "Camera Control".to_string(),
                value: format!("{}: {}", id, control),
                error: why.to_string(),
            })?
            .set_camera_control(id, control)
    }

    /// Will open the camera stream with set parameters. This will be called internally if you try and call [`frame()`](crate::Camera::frame()) before you call [`open_stream()`](crate::Camera::open_stream()).
    /// The callback will be called every frame.
    /// # Errors
    /// If the specific backend fails to open the camera (e.g. already taken, busy, doesn't exist anymore) this will error.
    pub fn open_stream(&mut self) -> Result<(), NokhwaError> {
        self.camera
            .lock()
            .map_err(|why| NokhwaError::SetPropertyError {
                property: "camera".to_string(),
                value: "callback".to_string(),
                error: why.to_string(),
            })?
            .open_stream()
    }

    /// Sets the frame callback to the new specified function. This function will be called instead of the previous one(s).
    pub fn set_callback(
        &mut self,
        callback: impl FnMut(Buffer) + Send + 'static,
    ) -> Result<(), NokhwaError> {
        *self
            .frame_callback
            .lock()
            .map_err(|why| NokhwaError::GetPropertyError {
                property: "frame_callback".to_string(),
                error: why.to_string(),
            })? = Box::new(callback);
        Ok(())
    }

    /// Polls the camera for a frame, analogous to [`Camera::frame`](crate::Camera::frame)
    /// # Errors
    /// This will error if the camera fails to capture a frame.
    pub fn poll_frame(&mut self) -> Result<Buffer, NokhwaError> {
        let frame = self
            .camera
            .lock()
            .map_err(|why| NokhwaError::ReadFrameError(why.to_string()))?
            .frame()?;
        *self.last_frame_captured.lock() = frame.clone();
        Ok(frame)
    }

    /// Gets the last frame captured by the camera.
    #[must_use]
    pub fn last_frame(&self) -> Buffer {
        self.last_frame_captured
            .lock()
            .map_err(|why| NokhwaError::ReadFrameError(why.to_string()))?
            .clone()
    }

    /// Checks if stream if open. If it is, it will return true.
    #[must_use]
    pub fn is_stream_open(&self) -> bool {
        self.camera
            .lock()
            .map_err(|why| NokhwaError::GetPropertyError {
                property: "is stream open".to_string(),
                error: why.to_string(),
            })?
            .is_stream_open()
    }

    /// Will drop the stream.
    /// # Errors
    /// Please check the `Quirks` section of each backend.
    pub fn stop_stream(&mut self) -> Result<(), NokhwaError> {
        self.camera
            .lock()
            .map_err(|why| NokhwaError::StreamShutdownError(why.to_string()))
            .stop_stream()
    }
}

impl Drop for CallbackCamera {
    fn drop(&mut self) {
        let _stop_stream_err = self.stop_stream();
        self.die_bool.store(true, Ordering::SeqCst);
    }
}

fn camera_frame_thread_loop(
    camera: &AtomicLock<Camera>,
    frame_callback: &HeldCallbackType,
    last_frame_captured: &AtomicLock<ImageBuffer<Rgb<u8>, Vec<u8>>>,
    die_bool: &Arc<AtomicBool>,
) {
    loop {
        if let Ok(img) = camera.lock().fr {
            *last_frame_captured.lock() = img.clone();
            if let Some(cb) = (*frame_callback.lock()).as_mut() {
                cb(img);
            }
        }
        if die_bool.load(Ordering::SeqCst) {
            break;
        }
    }
}
