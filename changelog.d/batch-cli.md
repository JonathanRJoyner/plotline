Section: Front ends

- **Batch runs in the command line:** `plotline run STYLE IMAGE...` takes several images, each a file or a folder (its PNG, JPEG and WebP files, sorted by name, so PowerShell needs no glob). `--out` and the other options that name an output file then name folders that get one file per image, `--gcode DIR` gets a subfolder per image, and every other option applies to every image. It prints a line per image as it finishes, a failing image does not stop the others, and the exit status is 1 if any failed. Two images with the same name are refused before anything runs.
