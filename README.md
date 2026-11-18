# Mimizan Lab X-Trans

> A Fuji X-Trans RAF in, a linear DNG out.

`xtrans2dng` turns the RAF of a Fuji X-Trans camera into a DNG that holds red, green and blue for every pixel. The mosaic is not interpolated. It is taken apart into the luminance and the two chrominances it carries on the tile's own frequencies, under the prior the mosaic itself makes most probable.

It is the companion of [Mimizan Lab](https://github.com/Draganito/mimizan), the darkroom that reads a Bayer mosaic the same way. Mimizan Lab does not read X-Trans; from 0.19.0 it opens the DNG this program writes. Any raw developer that reads a linear DNG opens it too.

```bash
xtrans2dng DSCF3861.RAF
```

writes `DSCF3861.dng` beside the RAF.

## What comes out

A DNG with three 16-bit channels a pixel, in the camera's own colours:

- **Linear.** No tone curve, no colour profile applied, no sharpening, no noise reduction, no lens correction. The separation runs under the sRGB curve, where the noise is nearly the same at every level, and each channel is taken back to linear before it is written.
- **Not balanced.** The channels are the sensor's. The camera's white balance is in the file as the as-shot neutral, with the colour matrices for daylight and tungsten, so a raw developer balances and profiles the DNG as it would the RAF.
- **The camera's data.** Exposure, lens, orientation and the camera's own JPEG as the preview are carried over.
- **Black at 0, white at 65535.** The RAF's black level is taken off. What lay below black in the noise is cut to black (0.1 % of the values of a frame at ISO 3200).
- **Nearly the whole frame.** The frame ends on symmetry lines of the tile, so at most seven columns and seven rows of the RAF's picture are left out: 6238 x 4159 of 6240 x 4160, 5998 x 3997 of 6000 x 4000.
- **Losslessly compressed**, about 80 to 115 MB for 24 and 26 megapixels. `--uncompressed` writes it half as large again.

## Get it

Debian and Ubuntu, 64-bit Intel and AMD: [mimizan-lab-xtrans_0.1.0-1_amd64.deb](https://github.com/Draganito/mimizan-lab-xtrans/releases/download/v0.1.0/mimizan-lab-xtrans_0.1.0-1_amd64.deb). Install the downloaded file:

```bash
sudo apt install ./mimizan-lab-xtrans_0.1.0-1_amd64.deb
```

That installs the `xtrans2dng` command. Other builds are on the [releases page](https://github.com/Draganito/mimizan-lab-xtrans/releases) when there are any; for now the Debian package is the only one.

From the source, with Rust 1.88 or later:

```bash
cargo build --release
target/release/xtrans2dng --help
```

It has been built and run on Linux only. Nothing in it is specific to Linux, but Windows and macOS were not tried.

## Use it

```
xtrans2dng [OPTIONS] <RAF>...
```

Every RAF gives a DNG of its name beside it. A DNG that is there already is left alone and the file counts as not converted.

| Option | What it does |
| --- | --- |
| `-o`, `--out <PATH>` | Where the DNG goes: a directory, or with one RAF a file name. |
| `-f`, `--force` | Write over a DNG that is already there. |
| `--keep-highlights` | Leave burnt-out areas as recorded (see below). |
| `--uncompressed` | Write the picture uncompressed. |
| `-q`, `--quiet` | Say nothing but errors. |

While it works it says what it found: the camera, the prior, the share of burnt-out photosites, the times. A DNG is written under another name and renamed when it is whole, so a DNG that is there is finished. When a file fails the others are still converted, and the command ends with an error.

## What it takes

A 26-megapixel file takes about three quarters of a minute and close to 8 GB of memory: 43 to 45 s and a peak of 7.8 GB on a Ryzen 7 5700U (eight cores). Files are converted one after the other, so the memory is that of one file. A machine with 8 GB does not have that much free.

## Highlights

Where a channel is burnt out, the sensor says nothing about the colour. By default the balanced mosaic is cut at the level the first channel burns out at: a burnt-out area comes out flat and neutral, and no false colour spreads from it into what lies around it.

`--keep-highlights` leaves the mosaic as recorded there. The channels that still hold something above that level stay in the DNG, for a raw developer's highlight recovery. The prior is found on the cut mosaic either way.

## Where the DNG opens

Tried with the files of two cameras, an X-Pro3 (26 megapixels, at ISO 160 and ISO 3200) and an X-E3 (24 megapixels, a compressed RAF with a burnt-out sun):

- **darktable 5.0.1** and **RawTherapee 5.11** show the picture of the RAF.
- **LibRaw** (`dcraw_emu`) does too, and reads the compressed DNG to the same values as the uncompressed one.
- **Mimizan Lab 0.19.0** opens it as a colour file with three channels: the monochrome negative, the plate and the print are those of a Bayer file, without a separation step.

The DNG says black is 0 rather than carrying the RAF's black level, because darktable 5.0 does not read that tag for a picture of three channels and showed such a file grey and magenta.

## What is not known

- **How it stands against other X-Trans demosaicers.** It was not compared with any of them. What was measured is the separation against the picture a synthetic mosaic was made from, and against the Bayer separation of Mimizan Lab on the same pictures.
- **Other cameras.** Only the two above were tried. The file is decoded by [rawler](https://github.com/dnglab/dnglab); a RAF it reads and whose tile is an X-Trans one should convert, and a file with another mosaic is refused ("not an X-Trans mosaic").
- **Other raw developers** than the three above.

## How it works

The 6 x 6 tile has green on five of nine photosites, and red and blue exchanging places on the rest. Read as a signal, the mosaic is a luminance with two chrominances on carriers: green against red and blue on eight of them, red against blue on four. No carrier sits at Nyquist, and they lie closer together than a Bayer mosaic's, so a filter around each carrier, which is how a Bayer mosaic comes apart, loses the colour's detail here.

Instead the luminance and both chrominances of the 36 frequencies the tile ties together are solved at once, by least squares under a prior: how fast luminance and colour fall with frequency, how much colour there is, how much noise, how soft the lens. That prior is searched on the mosaic itself. Twenty-one priors are then weighed against each other pixel by pixel by how well each explains the mosaic around it: more or less colour, pictures that vary along one direction only, which is what a straight edge is, and lines near the sensor's limit, which otherwise leave the dots of the tile along them.

A second step follows where the picture has two colours. There every photosite measures the same quantity through its own gain and the colour is as sharp as the mosaic; this takes out the dots the first step leaves along a hard edge of colour.

The method was grown as an experiment beside Mimizan Lab. What was tried, what was left behind and why is written at the top of [`src/xtrans.rs`](src/xtrans.rs).

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).
