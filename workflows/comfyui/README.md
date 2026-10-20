# ComfyUI workflow templates

These are ComfyUI **API-format** workflows (export with *Save (API Format)* in ComfyUI).
PhotoKit loads the template whose file name matches the operation:

| File | Operation |
|---|---|
| `text_to_image.json` | Text to image |
| `image_to_image.json` | Image to image |
| `inpaint.json` | Inpainting (white mask = repaint) |
| `outpaint.json` | Extend the canvas |
| `upscale.json` | Model-based upscaling |
| `remove_background.json` | Not bundled — add your own (needs a custom node) |

String values that are exactly a placeholder are replaced with a typed value:
`{{prompt}}`, `{{negative}}`, `{{checkpoint}}`, `{{seed}}`, `{{steps}}`, `{{cfg}}`,
`{{width}}`, `{{height}}`, `{{batch}}`, `{{denoise}}`, `{{image}}`, `{{mask}}`,
`{{upscale_model}}`, `{{pad_left}}`, `{{pad_top}}`, `{{pad_right}}`, `{{pad_bottom}}`.

To customize, copy a file into the folder set by `comfyui.workflow_dir` in the
PhotoKit config; it overrides the built-in template of the same name. An
operation is reported as available only when every `class_type` in its template
exists on the connected ComfyUI server.
