package sort

import (
	"image"
	"image/color"
	"image/draw"
	stdsort "sort"

	"github.com/hanneskrug/pixelsort/cmd"
)

// Span is one row or one column, described by the fixed coordinate it sits on
// and the half-open range of the coordinate that varies along it.
type Span struct {
	Axis  cmd.Axis
	Fixed int
	From  int
	To    int
}

// Run is a half-open range of positions along a Span whose pixels all passed
// classifyRun.
type Run struct {
	From int
	To   int
}

// Len returns how many pixels the run covers.
func (r Run) Len() int { return r.To - r.From }

// Sorter writes the sorted pixels of every run back into the image.
type Sorter func(img *image.RGBA, span Span, runs []Run)

// order says which end of the luminance range a sorted run starts from.
type order bool

const (
	darkFirst   order = false
	brightFirst order = true
)

// NewCanvas copies src into a mutable RGBA image anchored at the origin.
func NewCanvas(src image.Image) *image.RGBA {
	b := src.Bounds()
	dst := image.NewRGBA(image.Rect(0, 0, b.Dx(), b.Dy()))
	draw.Draw(dst, dst.Bounds(), src, b.Min, draw.Src)
	return dst
}

// Spans enumerates every row or column of bounds along the chosen axis.
func Spans(bounds image.Rectangle, axis cmd.Axis) []Span {
	var outer, from, to int
	if axis == cmd.Horizontal {
		outer, from, to = bounds.Dy(), bounds.Min.X, bounds.Max.X
	} else {
		outer, from, to = bounds.Dx(), bounds.Min.Y, bounds.Max.Y
	}

	spans := make([]Span, 0, outer)
	for i := 0; i < outer; i++ {
		spans = append(spans, Span{Axis: axis, Fixed: i, From: from, To: to})
	}
	return spans
}

// SorterFor picks the sorter that walks pixels in the direction the axis
// implies, bound to the requested output order.
func SorterFor(axis cmd.Axis, reverse bool) Sorter {
	dir := darkFirst
	if reverse {
		dir = brightFirst
	}
	if axis == cmd.Horizontal {
		return func(img *image.RGBA, span Span, runs []Run) {
			sortRows(img, span, runs, dir)
		}
	}
	return func(img *image.RGBA, span Span, runs []Run) {
		sortColumns(img, span, runs, dir)
	}
}

func (s Span) at(img *image.RGBA, pos int) (int, int) {
	if s.Axis == cmd.Horizontal {
		return pos, s.Fixed
	}
	return s.Fixed, pos
}

// DecideIntervals walks the span once and closes a run every time a pixel
// falls outside the luminance band.
func DecideIntervals(img *image.RGBA, span Span, lower, upper float64) []Run {
	b := band{lower: lower, upper: upper}

	var (
		runs  []Run
		start = -1
	)
	for pos := span.From; pos < span.To; pos++ {
		x, y := span.at(img, pos)
		if classifyRun(img, x, y, b) {
			if start < 0 {
				start = pos
			}
			continue
		}
		if start >= 0 {
			runs = append(runs, Run{From: start, To: pos})
			start = -1
		}
	}
	if start >= 0 {
		runs = append(runs, Run{From: start, To: span.To})
	}
	return runs
}

// FilterShortRuns drops runs below minRun pixels. Sorting two or three pixels
// costs as much as sorting a hundred once the slice allocation is counted, and
// the result is invisible.
func FilterShortRuns(runs []Run, minRun int) []Run {
	kept := runs[:0]
	for _, r := range runs {
		if r.Len() >= minRun {
			kept = append(kept, r)
		}
	}
	return kept
}

func arrange(pixels []color.RGBA, dir order) {
	stdsort.Stable(newByLuminance(pixels))
	if dir == brightFirst {
		for i, j := 0, len(pixels)-1; i < j; i, j = i+1, j-1 {
			pixels[i], pixels[j] = pixels[j], pixels[i]
		}
	}
}

func sortRows(img *image.RGBA, span Span, runs []Run, dir order) {
	for _, r := range runs {
		pixels := make([]color.RGBA, 0, r.Len())
		for x := r.From; x < r.To; x++ {
			pixels = append(pixels, img.RGBAAt(x, span.Fixed))
		}
		arrange(pixels, dir)
		for i, p := range pixels {
			img.SetRGBA(r.From+i, span.Fixed, p)
		}
	}
}

func sortColumns(img *image.RGBA, span Span, runs []Run, dir order) {
	for _, r := range runs {
		pixels := make([]color.RGBA, 0, r.Len())
		for y := r.From; y < r.To; y++ {
			pixels = append(pixels, img.RGBAAt(span.Fixed, y))
		}
		arrange(pixels, dir)
		for i, p := range pixels {
			img.SetRGBA(span.Fixed, r.From+i, p)
		}
	}
}
