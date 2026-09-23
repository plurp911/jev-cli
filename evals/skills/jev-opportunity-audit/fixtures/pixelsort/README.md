# pixelsort

A command-line pixel sorter. It reads a PNG or JPEG, gathers contiguous runs of
pixels whose luminance falls inside a band, sorts each run by luminance, and
writes the result back out.

## Install

```
go install github.com/hanneskrug/pixelsort@latest
```

## Use

```
pixelsort -in photo.jpg -out sorted.png
pixelsort -in photo.jpg -out sorted.png -axis column -threshold 0.35 -ceiling 0.8
pixelsort -in photo.jpg -out sorted.jpg -reverse -min-run 12 -quality 85
```

## Flags

| Flag | Default | Meaning |
| --- | --- | --- |
| `-in` | | Path of the image to read. Required. |
| `-out` | | Path of the image to write. Required. |
| `-axis` | `row` | Gather runs along `row` or down `column`. |
| `-threshold` | `0.25` | Lowest luminance a pixel may have and still join a run. |
| `-ceiling` | `0.90` | Highest luminance a pixel may have and still join a run. |
| `-min-run` | `4` | Runs shorter than this are left untouched. |
| `-reverse` | off | Write each sorted run back from bright to dark. |
| `-quality` | `92` | JPEG quality. Ignored when the output is a PNG. |

## How it works

Luminance is the Rec. 709 weighted sum of the red, green and blue channels,
scaled into `0..1`. Every row or column is walked once; a run opens at the
first pixel inside the band and closes at the first pixel outside it. Fully
transparent pixels never join a run, because moving them would tint whatever
they are composited over.

Short runs are filtered out before sorting. Sorting three pixels costs about as
much as sorting a hundred once the slice allocation is counted, and the result
is invisible, so `-min-run` pays for itself on noisy photographs.

The sort is stable, so pixels of equal luminance keep their original order and
flat regions do not shimmer.

## Notes

The decoder reads the format from the file contents rather than the extension,
and refuses anything larger than 64 megapixels. The encoder writes to a
temporary file in the destination directory and renames it into place, so an
interrupted run leaves the previous output intact.

## License

MIT.
