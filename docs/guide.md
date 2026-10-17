# First plot guide

This guide takes you from a picture to a plot on paper. It then lists what to change when the plot is not what you expected. It is for anyone with a pen plotter: a purpose-built one, a 3D printer fitted with a pen, or a drawing robot. You need no experience with plotline.

Open the [hosted page](https://jonathanrjoyner.github.io/plotline/), or run `plotline gui` (see the [README](../README.md#command-line)). Your image is never uploaded: the engine runs inside the page.

- [From picture to plot](#from-picture-to-plot)
- [The same from the command line](#the-same-from-the-command-line)
- [Troubleshooting](#troubleshooting)

## From picture to plot

1. **Open an image.** Click Open image in the toolbar, or drop or paste a PNG, JPEG or WebP onto the page. Sample image opens a still life to try things on. Once an image is open, Suggestions at the top of the right-hand column lists changes that would suit it, and nothing changes until you click Apply.

2. **Pick a style and a look.** Click the Style button to open the style picker, which draws every style small on your image, and click one; the [style guide](styles.md) says what each is for. Under the button, the Look select starts the style from Balanced (its defaults), Faithful (follows the image's tones most closely), Quick (about half an hour) or Fine (the densest marks that still plot cleanly). A style with only Balanced shows no Look select. The style's own settings are in the left column, and a change redraws on its own; Reset to defaults puts them all back.

3. **Set the page.** In the Page panel choose the Size, Orientation and Margin of the paper in your plotter. Fit keeps the whole image on the page and fill covers the drawing area and crops the rest. Paper sets the colour the preview draws the ink onto, and does not change the plot.

4. **Set the pens.** Click Edit pens in the Pens panel to open the Pens dialog.
   - Pen bank: pick a maker's pen at its published line width, or type your own Line width. Choose an ink colour.
   - My pens: Save this pen keeps the chosen pen in this browser, to pick again later. Export and Import move your pens as a file.
   - Pens plot in list order, so list light inks first. Move up and Move down change the order.
   - Pen use, in the Pens panel, shares the drawing between the pens: "one pen" (the first alone, the default), "light to dark" or "by colour".
   - For a colour image, Apply on the colour suggestion in Suggestions picks two to four ink pens (from My pens if you saved any) and sets Pen use to "by colour".

5. **Read the stats, the tone match and the plot lint.** Under the preview, the stats give the path count, the pen-down and pen-up lengths and the plot time. The tone error (0 is a perfect match) and SSIM (1 is identical) say how closely the drawing's tones follow the image. The plot lint lists trouble it found on paper as an error, a warning or an info, and Apply changes the setting that fixes one.

6. **Fit and optimize.** Open Fit and optimize under the stats. To meet a budget, type a Target, choose its unit (min, paths, m or dots) and click Fit to target, which changes the style's density setting until the drawing is near it. To follow the image's tones best for the plot time, click Optimize with no target, then Apply to take the density it suggests.

7. **Plot the test chart.** Test chart in the Pens panel replaces the drawing with a page of labelled patches for the first pen, and needs no image. Plot it once for each pen and paper: it shows where lines merge, how big dots come out and how many passes the paper takes, and you can time the plot against the estimate. To chart another pen, move it to the top of the list, then click Run to draw your image again.

8. **Export.** There are three ways.
   - SVG: click Export SVG in the toolbar. It writes the page in millimetres with one layer per pen, for your plotter's own software.
   - G-code: click Settings in the toolbar, choose New machine under Machine and replace its placeholder values with your machine's own: the drawable area, the feeds, the pen heights, and the start and end G-code. The Export panel then has G-code buttons, one file per pen. Home the machine before you mount the pen, because the files have no `G28`, and check the pen-down Z first, because a wrong one can press the pen into the bed.
   - HPGL: click HPGL in the Export panel for one file with a pen selected for each layer. Settings > HPGL sets the plotter units per mm, 40 unless your plotter's manual says otherwise.

9. **Save a preset.** Save preset in the Presets panel writes the style, page, pens, settings and machine to a small JSON file. Load preset reads it back, and also reads an exported SVG or JSON drawing, which carries its preset.

## The same from the command line

From a clone, put `uv run` in front of `plotline`, as the [README](../README.md#command-line) shows.

Run a style from one of its looks, with a pen of your own, and write a preview to check. `plotline describe waves` lists the looks of a style.

```sh
plotline run waves photo.jpg --out drawing.svg --preview preview.png --look faithful --pen "Fine:0.3:black"
```

Fit the style's density setting to a plot time in minutes. `--fit-paths`, `--fit-metres` and `--fit-dots` fit the other budgets.

```sh
plotline run waves photo.jpg --out drawing.svg --fit-minutes 60
```

Write one G-code file per pen for a machine. The machine file is a machine's JSON, or a preset that has one, such as a preset you saved in the GUI after you entered the machine in Settings. `--machine` also makes the page the machine's drawable area.

```sh
plotline run waves photo.jpg --out drawing.svg --machine my-machine.json --gcode plots/
```

Write the test chart for a pen, to plot and check.

```sh
plotline testchart --out chart.svg --pen "Fine:0.3:black"
```

`plotline run` prints the path count and the estimated plot time, then the tone error and SSIM. The plot lint's findings follow on stderr, each with the `--set` options that fix it where a setting does. `--strict` makes a warning or an error exit with status 1. `--save-preset FILE` keeps the setup, and `--preset FILE` starts from one.

## Troubleshooting

Settings are named as their stage and key, such as `order.min_length_mm`, as in the [style guide](styles.md). The GUI shows each under its own label, which this guide gives where it differs.

### The drawing looks too light or faint

The marks cover too little of the paper. The plot lint may say "Where the image is darkest, the drawing covers only N % of the paper."

- Click Apply on that finding, where it has one. It sets closer lines, or more points or strokes, where that is sure to darken the drawing.
- Or lower the style's density setting by hand, such as `waves.spacing_mm`, or raise the count, such as `voronoi.count`. The style guide marks each style's density setting.
- Raise Gamma (`tone.gamma`) in Image: above 1 it darkens the midtones. For a washed-out image set Levels (`tone.levels`) to auto, which sets black and white from the image.
- Try the Faithful look, or set Response (`tone.response`) to auto, which corrects the image for how much paper the style covers at each grey.
- If the lint says "No marks at these settings", lower the style's threshold (such as `points.threshold`) or change the tone settings it names.

### The plot would take too long

There are too many marks, pen lifts or moves for the machine's speed.

- Type the minutes you can spare as the Target in Fit and optimize and click Fit to target. On the command line use `--fit-minutes`.
- Choose the Quick look, where the style has one.
- Choose a style that draws long strokes and lifts the pen rarely: `waves`, `spiral`, `line`, `scribble`, `hilbert`, `contours` or `flow`. The lint says "Pen lifts take N % of the plot time" when lifting is the cost.
- If the lint says "Pen-up travel takes N % of the plot time", Apply turns on Path order's Optimize (`order.optimize`), which cuts the travel.
- Choose a smaller Size in the Page panel.
- Check the speeds behind the estimate. With no machine they are Draw, Travel and Pen lift in Settings > Plot time; with a machine they come from its feeds and its pen lift. Time the test chart's plot against its estimate.

### Many tiny specks and pen lifts

Very short paths each cost a pen lift. The lint says "N paths are shorter than the pen is wide; each costs a pen lift for a speck."

- Click Apply on that finding. It sets Path order's Min length (`order.min_length_mm`) to about the pen's width, so the shorter paths are dropped.
- On `grid` and `halftone`, raise `points.threshold` to leave out the dots in the lightest areas.
- If the lifts themselves are the problem, choose a style of long strokes, as under "The plot would take too long".

### Lines drawn over each other, or ink wearing through the paper

The pen passes the same spot too often. The lint says "N % of what pen X inks is drawn over 3 or more times", or "Marks N mm apart are closer than the W mm pen, so they overlap." Two passes, as where hatching crosses, are not flagged.

- Enter the pen's real Line width in the Pens dialog. The lint and the preview both use it.
- Click Apply on the finding. For `hatch` it takes one level off (`hatch.levels`), for most other styles it raises the spacing by half, and for marks closer than the pen it sets the spacing to the pen's width.
- On `halftone`, keep `stipple.size_mm` no larger than `points.spacing_mm`, because dots wider than the spacing overlap whatever the spacing.
- `scribble` and `sketch` cross their own lines by design. Use a pen that does not bleed or pill the paper, or choose another style.
- The test chart's PASSES patches show how many passes your paper and pen take.

### Colours come out muddy, or one pen does everything

There are two causes.

- Pen use is "one pen", the default, so only the first pen draws. The plot lint notes it, and its Apply sets "light to dark". For colours, set "by colour" in the Pens panel.
- The pens do not match the image. With "by colour", each pen draws the part of the image its colour makes up. Apply the colour suggestion in Suggestions, which picks two to four inks that draw the image's colours best. The lint says "These pens draw nothing at these settings" for a pen whose colour the image lacks, and you can remove that pen.

### Long straight moves curve on a hanging plotter

Some firmware draws a long straight move as a curve. Set Settings > G-code > Longest move (`max_segment_mm` in a machine file). plotline then cuts every move into pieces no longer than it. The least allowed is 0.5 mm, and a shorter length means more lines in the file. Empty keeps moves whole. It changes G-code only, not SVG or HPGL.

### A servo or solenoid lifts the pen

The pen is lifted by a servo or a solenoid, not by a Z move. In Settings > Pen lift, set Lift by to "G-code lines" (`"pen_lift": "gcode"` in a machine file). Then write your machine's own lines in Pen down and Pen up (`pen_down_gcode` and `pen_up_gcode`). plotline writes them as they are, where it would move Z. The servo command it fills in is a placeholder, so replace it. Settle (`settle_ms`) is the pause after each pen move, so the pen has landed or lifted before the next move; set it to the time your servo or solenoid needs.

### The plot does not line up with the page

- Set Size, or Width and Height, in the Page panel to the paper in the plotter. Margin keeps that border clear: the drawing area is the page less the margin. The SVG has exactly that page size in millimetres, so give your plotter's software the same paper size and no scaling.
- The lint reports a drawing that leaves the page ("vertices lie off the page") or goes into the margin ("vertices lie in the margin"). It also reports a page larger than the machine's drawable area, and its Apply turns the page when the turned page fits.
- For G-code, the page is centred in the machine's drawable area, which you set in Settings > Machine as Width, Height, Offset X and Offset Y. Offset is the machine position of the area's corner with the smallest X and Y. To shift the whole plot, change Offset X and Offset Y by the shift you want. Picking a machine also sets the page to its drawable area.
- Home the machine before you mount the pen. The G-code has no `G28`, so the plot sits where the machine's own zero is.
- Plot the test chart first. It starts at the drawing area's top-left corner, so a short plot shows where the page lands before a long one.
