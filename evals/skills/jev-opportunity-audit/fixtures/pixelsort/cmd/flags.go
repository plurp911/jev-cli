// Package cmd parses the command line into the options the pipeline needs.
package cmd

import (
	"errors"
	"flag"
	"fmt"
	"io"
)

// Axis selects whether runs are gathered across rows or down columns.
type Axis int

const (
	// Horizontal gathers runs left to right within a single row.
	Horizontal Axis = iota
	// Vertical gathers runs top to bottom within a single column.
	Vertical
)

// Options holds every value the pipeline reads after parsing finishes.
type Options struct {
	Input   string
	Output  string
	Axis    Axis
	Lower   float64
	Upper   float64
	MinRun  int
	Reverse bool
	Quality int
}

// Parse turns raw arguments into Options, rejecting any combination that the
// later stages cannot act on.
func Parse(args []string) (Options, error) {
	var (
		opts Options
		axis string
	)

	fs := flag.NewFlagSet("pixelsort", flag.ContinueOnError)
	fs.SetOutput(io.Discard)
	fs.StringVar(&opts.Input, "in", "", "path of the image to read")
	fs.StringVar(&opts.Output, "out", "", "path of the image to write")
	fs.StringVar(&axis, "axis", "row", "sort along `row` or `column`")
	fs.Float64Var(&opts.Lower, "threshold", 0.25, "lowest luminance a pixel may have and still join a run")
	fs.Float64Var(&opts.Upper, "ceiling", 0.90, "highest luminance a pixel may have and still join a run")
	fs.IntVar(&opts.MinRun, "min-run", 4, "runs shorter than this many pixels are left untouched")
	fs.BoolVar(&opts.Reverse, "reverse", false, "write each sorted run back from bright to dark")
	fs.IntVar(&opts.Quality, "quality", 92, "JPEG quality, ignored for PNG output")

	if err := fs.Parse(args); err != nil {
		return Options{}, err
	}

	switch axis {
	case "row":
		opts.Axis = Horizontal
	case "column":
		opts.Axis = Vertical
	default:
		return Options{}, fmt.Errorf("unknown axis %q, want row or column", axis)
	}

	return opts, opts.validate()
}

func (o Options) validate() error {
	switch {
	case o.Input == "":
		return errors.New("missing -in")
	case o.Output == "":
		return errors.New("missing -out")
	case o.Lower < 0 || o.Lower > 1:
		return fmt.Errorf("threshold %.3f outside 0..1", o.Lower)
	case o.Upper < 0 || o.Upper > 1:
		return fmt.Errorf("ceiling %.3f outside 0..1", o.Upper)
	case o.Lower >= o.Upper:
		return fmt.Errorf("threshold %.3f must be below ceiling %.3f", o.Lower, o.Upper)
	case o.MinRun < 1:
		return fmt.Errorf("min-run %d must be at least 1", o.MinRun)
	case o.Quality < 1 || o.Quality > 100:
		return fmt.Errorf("quality %d outside 1..100", o.Quality)
	}
	return nil
}
