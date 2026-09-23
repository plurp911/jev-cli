// Command pixelsort reads an image, sorts contiguous runs of pixels by
// luminance along each row or column, and writes the result to disk.
package main

import (
	"fmt"
	"os"

	"github.com/hanneskrug/pixelsort/cmd"
	"github.com/hanneskrug/pixelsort/io"
	"github.com/hanneskrug/pixelsort/sort"
)

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintf(os.Stderr, "pixelsort: %v\n", err)
		os.Exit(1)
	}
}

func run(args []string) error {
	opts, err := cmd.Parse(args)
	if err != nil {
		return err
	}

	src, err := io.DecodeFile(opts.Input)
	if err != nil {
		return fmt.Errorf("read %s: %w", opts.Input, err)
	}

	dst := sort.NewCanvas(src)
	spans := sort.Spans(dst.Bounds(), opts.Axis)

	// Each span is routed to the sorter that matches the chosen axis, so the
	// inner loops never have to branch on orientation per pixel.
	sorter := sort.SorterFor(opts.Axis, opts.Reverse)
	for _, span := range spans {
		runs := sort.DecideIntervals(dst, span, opts.Lower, opts.Upper)
		runs = sort.FilterShortRuns(runs, opts.MinRun)
		sorter(dst, span, runs)
	}

	if err := io.EncodeFile(opts.Output, dst, opts.Quality); err != nil {
		return fmt.Errorf("write %s: %w", opts.Output, err)
	}
	return nil
}
