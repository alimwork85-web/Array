# Face models

Both models come from [OpenCV Zoo](https://github.com/opencv/opencv_zoo) and are embedded in the app (`include_bytes!` in `src/faces.rs`), so face sorting runs fully offline.

| File | Purpose | License | SHA-256 |
|---|---|---|---|
| `face_detection_yunet_2023mar.onnx` | Face detection + 5 landmarks (YuNet) | MIT | `8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4` |
| `face_recognition_sface_2021dec.onnx` | 128-d face embedding (SFace) | Apache 2.0 | `0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79` |

Sources:
- https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet
- https://github.com/opencv/opencv_zoo/tree/main/models/face_recognition_sface
