# Media fixtures

These original fixtures were generated locally with Pillow from a solid RGB image:
`Image.new("RGB", (2, 3), (100, 150, 200))`, saved as PNG, JPEG, and WebP.
No third-party image or fixture was copied. Lossy formats may alter the exact color.

The files exercise MIME detection, dimensions, base64 handling, and media bounds
using small, valid containers. The tests also mutate their headers to cover hostile
input without allocating large decoded images.
