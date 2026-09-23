// Package sort rearranges pixels within an image by their relative luminance.
package sort

import (
	"image"
	"image/color"
)

// Rec709 weights, applied to colour channels that have already been scaled
// into the unit interval.
const (
	redWeight   = 0.2126
	greenWeight = 0.7152
	blueWeight  = 0.0722
)

// band is the closed luminance interval a pixel must fall inside to take part
// in a run.
type band struct {
	lower float64
	upper float64
}

// luminanceScore returns the perceived brightness of c on a 0..1 scale.
func luminanceScore(c color.Color) float64 {
	r, g, b, _ := c.RGBA()
	const full = 0xffff
	return redWeight*float64(r)/full +
		greenWeight*float64(g)/full +
		blueWeight*float64(b)/full
}

// classifyRun reports whether the pixel at (x, y) belongs inside a run. A
// pixel that is fully transparent is skipped whatever its luminance, because
// compositing it back would tint whatever sits underneath.
func classifyRun(img *image.RGBA, x, y int, b band) bool {
	c := img.RGBAAt(x, y)
	if c.A == 0 {
		return false
	}
	score := luminanceScore(c)
	return score >= b.lower && score <= b.upper
}

// byLuminance orders a slice of pixels from dark to bright. Ties keep their
// original left-to-right order so that flat regions do not shimmer between
// runs of the same image.
type byLuminance struct {
	pixels []color.RGBA
	scores []float64
}

func (s byLuminance) Len() int { return len(s.pixels) }

func (s byLuminance) Less(i, j int) bool { return s.scores[i] < s.scores[j] }

func (s byLuminance) Swap(i, j int) {
	s.pixels[i], s.pixels[j] = s.pixels[j], s.pixels[i]
	s.scores[i], s.scores[j] = s.scores[j], s.scores[i]
}

// newByLuminance precomputes a score per pixel so the comparison function
// stays cheap across the O(n log n) swaps that follow.
func newByLuminance(pixels []color.RGBA) byLuminance {
	scores := make([]float64, len(pixels))
	for i, p := range pixels {
		scores[i] = luminanceScore(p)
	}
	return byLuminance{pixels: pixels, scores: scores}
}
