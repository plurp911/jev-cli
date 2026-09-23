// Package io reads and writes the image formats pixelsort supports.
package io

import (
	"bufio"
	"fmt"
	"image"
	"os"
	"path/filepath"
	"strings"

	_ "image/jpeg"
	_ "image/png"
)

// maxPixels caps the total pixel count so a malformed header cannot make the
// decoder reserve gigabytes before it fails.
const maxPixels = 64 << 20

// DecodeFile reads an image from path. The format comes from the file
// contents rather than the extension, so a mislabelled file still decodes.
func DecodeFile(path string) (image.Image, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	r := bufio.NewReader(f)
	cfg, format, err := image.DecodeConfig(r)
	if err != nil {
		return nil, fmt.Errorf("inspect header: %w", err)
	}
	if cfg.Width <= 0 || cfg.Height <= 0 {
		return nil, fmt.Errorf("degenerate %s image %dx%d", format, cfg.Width, cfg.Height)
	}
	if int64(cfg.Width)*int64(cfg.Height) > maxPixels {
		return nil, fmt.Errorf("%dx%d exceeds the %d pixel limit", cfg.Width, cfg.Height, maxPixels)
	}

	if _, err := f.Seek(0, 0); err != nil {
		return nil, err
	}
	r.Reset(f)

	img, _, err := image.Decode(r)
	if err != nil {
		return nil, fmt.Errorf("decode %s: %w", format, err)
	}
	return img, nil
}

// formatFromExtension maps an output path to the encoder that writes it.
func formatFromExtension(path string) (string, error) {
	switch strings.ToLower(filepath.Ext(path)) {
	case ".png":
		return "png", nil
	case ".jpg", ".jpeg":
		return "jpeg", nil
	default:
		return "", fmt.Errorf("cannot write %q, want .png, .jpg or .jpeg", filepath.Ext(path))
	}
}
