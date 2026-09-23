package io

import (
	"bufio"
	"fmt"
	"image"
	"image/jpeg"
	"image/png"
	"os"
	"path/filepath"
)

// EncodeFile writes img to path, choosing the encoder from the extension.
// The bytes land in a sibling temporary file first and are renamed into place
// afterwards, so an interrupted run never leaves a half-written image behind.
func EncodeFile(path string, img image.Image, quality int) error {
	format, err := formatFromExtension(path)
	if err != nil {
		return err
	}

	tmp, err := os.CreateTemp(filepath.Dir(path), ".pixelsort-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())

	w := bufio.NewWriter(tmp)
	switch format {
	case "png":
		enc := png.Encoder{CompressionLevel: png.BestCompression}
		err = enc.Encode(w, img)
	case "jpeg":
		err = jpeg.Encode(w, img, &jpeg.Options{Quality: quality})
	default:
		err = fmt.Errorf("unhandled format %q", format)
	}
	if err != nil {
		tmp.Close()
		return fmt.Errorf("encode %s: %w", format, err)
	}

	if err := w.Flush(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(tmp.Name(), path)
}
