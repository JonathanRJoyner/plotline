# Style guide

Every style turns the image's tone into pen marks in its own way. This guide shows each one on the same picture, says what it is good for, which settings matter and what turning them does, and how to pick a pen for it. New to plotline? The [first plot guide](guide.md) takes you from a picture to paper.

Each picture is the code-rendered still life from the README, drawn on A4 with a 0.3 mm black pen at the style's defaults. There is one exception: `ink` is drawn at a threshold of 0.3, because `ink` is made for line art and the still life is not. A setting is named as its stage and key, such as `hatch.spacing_mm`; the GUI shows the same settings under each stage's heading, `plotline describe STYLE` lists them with their ranges, and `plotline run` takes them as `--set stage.key=value`.

- [Settings every style shares](#settings-every-style-shares)
- [Lines](#lines): [`grid`](#grid), [`hatch`](#hatch), [`waves`](#waves), [`spiral`](#spiral), [`truchet`](#truchet), [`ink`](#ink), [`learned`](#learned)
- [The Voronoi point family](#the-voronoi-point-family): [`stipple`](#stipple), [`line`](#line), [`dashes`](#dashes), [`tree`](#tree), [`mesh`](#mesh), [`cells`](#cells), [`scribble`](#scribble)
- [Sketch lines](#sketch-lines): [`sketch`](#sketch)
- [Other placers](#other-placers): [`halftone`](#halftone), [`dither`](#dither), [`circles`](#circles)
- [Tone only](#tone-only): [`contours`](#contours), [`flow`](#flow), [`hilbert`](#hilbert), [`mosaic`](#mosaic)

The GUI's style picker groups the styles under these same headings, and a test keeps the two equal, so a new style gets a section here and a family there.

## Settings every style shares

**Image** (the `tone` stage) turns the picture into the map of ink the style draws from. `resolution` is the map's longer side in pixels; more follows the image more closely and runs slower. `black` and `white` clip the ends of the tone range, or `levels` set to `auto` picks them from the image itself. `gamma` above 1 darkens the midtones: `waves`, `spiral` and `mosaic` start at 1.9, because they ink in proportion to the tone and would draw the midtones too light at 1. `contrast` pushes tones away from mid-grey, `blur_mm` smooths away noise and fine texture, and `invert` swaps light and dark. `focus` is a list of points, `[x, y]` as shares of the image from its top-left corner and set in the GUI by turning on Focus and clicking the preview: the map keeps its detail within `focus_mm` of the nearest point and blurs more the further out it goes, up to `focus_blur_mm` from three radii out, so a style's marks spend their detail near the points. Without points nothing changes. `learned` has a Model stage in its place, which places the image on the page the same way.

**Pen use** (the `split` stage) shares the drawing between the pens: the first pen alone, light to dark in list order, or each pen by its colour. With several pens, every style but `ink` and `learned` spreads its marks so that one pen does not draw on top of another. By colour, the tone `gamma` darkens each pen's own share of a colour rather than adding grey, which the black pen would draw over every colour ([decision](decisions/2026-10-17-colour-lanes.md)). `waves`, `spiral` and `hilbert` lift a pen where its share of the ink is under half of what their lines cover over white, so a pen leaves paper and other pens' colours alone.

**Path order** (the `order` stage) joins the paths that meet into the fewest pen-down trails (`join`: `on` draws each edge once, `retrace` also draws some short edges twice so the pen lifts less; see [the decision](decisions/2026-10-17-join-paths-into-trails.md)), reorders the paths to cut pen-up travel (`optimize`), drops paths shorter than `min_length_mm`, which clears specks, and, with `simplify_mm` above 0, removes the vertices a pen cannot show (Ramer–Douglas–Peucker; see [the decision](decisions/2026-10-16-path-simplification.md)).

**Density.** Every style but `ink` and `learned` has one density setting, named in its section. Fit to target in the GUI, the `--fit-minutes`, `--fit-paths`, `--fit-metres` and `--fit-dots` flags and the MCP `fit` tool change that setting until the drawing meets a plot time, a path count, metres of line or a dot count.

**Looks.** Most styles offer a few starting points besides their defaults (Balanced), each chosen by measuring nine code-drawn pictures on A4 with a 0.3 mm pen and looking at the drawings: Faithful follows the image's tones most closely, Quick plots in about half an hour, and Fine draws the densest marks that still plot cleanly. No look draws its marks more than twice as sparsely as the defaults. A look sets only its few settings, so Pen use and the image settings stay. Pick one with Look under the style in the GUI, `--look` on the command line or `look` on the MCP `run` tool; [the looks decision](decisions/2026-10-16-looks.md) and [its re-measure](decisions/2026-10-17-looks-on-more-pictures.md) have the rules and every measurement.

**Pen lifts.** Each path costs one lift of the pen. On a plotter that lifts slowly, or on a pen-fitted 3D printer where every lift is a Z move, styles that draw a few long paths (`waves`, `spiral`, `line`, `scribble`, `hilbert`, `contours`, `flow`) plot faster and wear the machine less than styles of thousands of dots or strokes.

## Lines

### `grid`

<img src="images/grid.png" width="260" align="right" alt="The still life drawn with the grid style">

A filled dot at every point of a square grid where the ink reaches a threshold. Tone shows only as dots or no dots, so the result reads like a poster with one level.

**Use it for** bold, graphic pictures with clear shapes, and as a quick first plot to check the page and the pen.

**Settings**
- `points.spacing_mm` (density): the grid pitch. Finer spacing gives more detail and more, smaller dots.
- `points.threshold`: the least ink that gets a dot. Lower it to bring in the midtones.
- `render.size`: a dot's width as a share of the spacing. At 1, the default, neighbouring dots touch and a dark area is about four-fifths ink at any spacing; lower it for a lighter, more open pattern.

**Pens.** Each dot is filled with a spiral, so a wider pen fills it in fewer turns and plots faster. A dot no wider than the pen is one touch of the pen. Every dot is a pen lift.

**Looks:** Balanced only: a Quick sparse enough to save time turned the picture into a few large dots, and no finer spacing scored within a quarter of the default's.

### `hatch`

<img src="images/hatch.png" width="260" align="right" alt="The still life drawn with the hatch style">

Straight parallel lines over the areas dark enough, the engraver's technique. Each further level adds lines at another angle over darker tones, which is cross-hatching.

**Use it for** a classic pen-and-ink look, architecture and objects with flat areas of tone.

**Settings**
- `hatch.spacing_mm` (density): the distance between lines.
- `hatch.levels`: how many crossing layers, 1 to 4. Each turns by 180 / levels degrees and covers only darker ink.
- `hatch.threshold`: the least ink that gets lines. The levels spread evenly between it and full ink.
- `hatch.angle_deg`: the direction of the first layer.

**Pens.** Fineliners from 0.1 to 0.3 mm. Where layers cross the pen passes a spot twice, which most papers take well. Keep the spacing above the pen width, or the lines merge into a smear.

**Looks:** Balanced only: no spacing scores clearly better than the default, and the finer ones score worse.

### `waves`

<img src="images/waves.png" width="260" align="right" alt="The still life drawn with the waves style">

Parallel lines that ripple more where the image is darker, each line one stroke. The ripples carry the tone, so the lines never stop.

**Use it for** a calm, even texture with very few pen lifts; portraits and soft gradients read well.

**Settings**
- `waves.spacing_mm` (density): the distance between lines.
- `waves.amplitude`: how far a ripple swings over full ink, as a share of half the spacing. Lower is quieter and lighter.
- `waves.wavelength_mm`: the length of one ripple. Shorter ripples look darker and take longer to draw.
- `waves.angle_deg`: the direction of the lines.

**Pens.** Any pen that draws a steady line, ballpoints and gel pens included, since each line is one long stroke. A fine pen keeps the ripples apart.

**Looks:** Faithful (`tone.response` auto and `waves.spacing_mm` 1.7), Quick (4.0 mm) and Fine (1.0 mm).

### `spiral`

<img src="images/spiral.png" width="260" align="right" alt="The still life drawn with the spiral style">

One spiral from the centre out, rippling more where the image is darker. It is `waves` wound around the middle of the page.

**Use it for** round subjects and a single-stroke look that draws the eye to the centre.

**Settings**
- `spiral.spacing_mm` (density): the distance between turns.
- `spiral.amplitude` and `spiral.wavelength_mm`: as for `waves`.

**Pens.** As for `waves`. The spiral is one long stroke, broken only where it leaves the image.

**Looks:** Faithful (`tone.response` auto) and Fine (`spiral.spacing_mm` 1.2). No Quick: at twice the spacing the rings take over the picture.

### `truchet`

<img src="images/truchet.png" width="260" align="right" alt="The still life drawn with the truchet style">

Square tiles in random turns, each holding quarter-circle arcs about two opposite corners, so the arcs join from tile to tile into winding curves (Smith's Truchet tiles). Darker tiles nest more arcs, and the counts are spread by error diffusion so the tone stays even.

**Use it for** a decorative, pattern-first look; the picture shows through the pattern rather than in detail.

**Settings**
- `truchet.spacing_mm` (density): the distance between nested arcs. Closer arcs fit more into a tile and draw darker.
- `truchet.tile_mm`: the tile size. Smaller tiles show more of the picture; larger ones show more of the pattern.
- `truchet.seed`: picks another arrangement of turns. A change of tone never reshuffles them.

**Pens.** Keep `spacing_mm` above the pen width, or the nested arcs merge into bands; plot lint warns about it.

**Looks:** Faithful (`truchet.spacing_mm` 0.3, the default pen's width, so the arcs fill into bands).

### `ink`

<img src="images/ink.png" width="260" align="right" alt="The still life drawn with the ink style at threshold 0.3">

Draws the marks of line art, lettering and inked drawings as they are. Every ink shape is outlined and filled with rings, and strokes thinner than the pen get one line down the middle, so the drawing keeps its own shapes and weights. It starts from a finer tone map than the other styles (`tone.resolution` 2500) to keep hairlines.

**Use it for** scans of drawings, logos, lettering and comics. On a photograph it draws the dark areas as solid shapes, as in the picture.

**Settings**
- `ink.threshold`: the ink that counts as part of a shape. Lower it to keep faint lines; raise it to drop a grey background.
- `ink.mode`: `fill` outlines and fills each shape; `outline` draws the outline only; `centreline` draws one line down each stroke, for line art drawn in single strokes.
- `ink.spacing`: the gap between fill rings as a share of the pen width. Below 1 the rings overlap, so filled shapes plot solid.
- `ink.min_area_mm2`: drops shapes smaller than this, such as dust in a scan.

**Pens.** Set the pen you will plot with before running: the rings are spaced by its width. Large solid areas lay down a lot of ink, so use paper that takes it.

**Looks:** Balanced only: it has no density setting.

### `learned`

<img src="images/styles/learned.png" width="260" align="right" alt="The still life drawn with the learned style and its built-in model">

Draws the lines that a line model finds in the image. The model turns the picture into a map of where its lines are; the ink stage then draws each line once down its middle, as `ink` does in its centreline mode, so a line is one pen stroke.

**Use it for** a drawing that follows the edges and outlines of a photograph instead of its tones. Which lines you get depends on the model. The built-in one, `xdog`, is the default and needs no download: it is XDoG (H. Winnemöller, J. E. Kyprianidis and S. C. Olsen, "XDoG: an eXtended difference-of-Gaussians compendium including advanced image stylization", *Computers & Graphics* 36(6), 2012), which blurs the image at two widths, sharpens it by their difference and draws where the sharpened picture falls below a threshold, which at the defaults is the dark side of each outline. `teed-1`, a small neural network you can download, finds edges, texture included, rather than an artist's selection of lines.

**Getting a trained model.** Trained models are not part of plotline, and `learned` draws with the built-in model until you choose one. In the browser, Styles, then Get models, lists the models plotline offers, each card with its picture, size and licence; Download first shows the licence, the attribution, what the model was trained on, its source and its paper, and fetches nothing until you accept. Each model you have then appears in the Styles tab as an entry of its own. On the command line, `plotline models list` shows them and `plotline models get teed-1` shows the same and downloads it. The browser fetches the model runtime, a second WebAssembly module, when a model is first downloaded, loaded or restored, so a visit that never uses one does not download it. A model file of your own, an ONNX file of up to 200 MB, loads from Load your own in the browser or with `plotline models add`; you are responsible for its licence.

**Settings**
- `model.model`: the id of the model to draw with: `xdog`, the built-in one and the default, or a loaded one such as `teed-1`. A run with a model that is not loaded says how to get it.
- `model.resolution`: the long side, in pixels, of the image the model sees, from 256 to 2048. More finds finer lines and takes longer. A trained model works on tiles of 512 pixels, so memory stays low at any resolution.
- `model.sigma`: for the built-in model, the width in model pixels of the finer of its two blurs, from 0.3 to 8 (1 by default); the wider is 1.6 times it. Raise it for fewer, bolder lines.
- `model.strength`: for the built-in model, how strongly it sharpens edges into lines, from 1 to 250 (100 by default). Raise it to find fainter edges, at the cost of more texture and noise.
- `model.threshold`: for the built-in model, the brightness, after sharpening, below which it draws, from -1 to 1 (0.1 by default). Near 0 it draws outlines alone; raised towards the middle grey it also fills dark areas, which the centreline tracer then draws down their middles.
- `model.invert`: swaps line and paper in the model's answer. Only a model file of your own that draws white lines on black needs it; the listed models already run the right way.
- `ink.threshold`: how dark a line must be to be drawn. Lower it to keep faint lines; raise it to keep only the strong ones.

The model's answer is cached like any stage's, so changing the threshold, the pens or the path order does not run the model again. A few pixels of the model's answer can differ between the browser and the command line, so the two may not draw exactly the same paths.

**Pens.** Any pen; a line is drawn at the pen's own width. Each line costs one pen lift, and a busy photograph gives many short ones, so raise `ink.threshold`, or `order.min_length_mm`, to drop the fainter and shorter ones.

**Looks:** Balanced only: it has no density setting.

## The Voronoi point family

These styles share one placement of points: the points spread evenly while staying dense where the image is dark (weighted Voronoi relaxation). The relaxation is the slow part, so switching between these styles reuses the points and redraws at once. Their placement settings are the same everywhere:

- `voronoi.count` (the density for every style in the family): the number of points. More points give finer detail and a longer plot. It counts per pen: by colour the pens share their points by ink, so a pen whose colour makes up a tenth of the picture's ink places about a tenth of them, and light to dark each pen places its own.
- `voronoi.exponent`: density follows ink to this power. A line through the points darkens with the square root of their density, so `line`, `scribble`, `tree`, `mesh` and `cells` default to 2, and the dot styles `stipple` and `dashes` to 1 ([decision](decisions/2026-10-17-ink-squared-for-joined-points.md)). A switch between a style at 1 and one at 2 places the points again.
- `voronoi.iterations`: relaxation passes. More spread the points more evenly.
- `voronoi.seed`: picks another starting scatter.

### `stipple`

<img src="images/styles/stipple.png" width="260" align="right" alt="The still life drawn with the stipple style">

A dot at each point, sized by the ink under it.

**Use it for** the classic stippled engraving look, and subjects with soft shading.

**Settings**
- `stipple.size_mm`: the dot diameter over full ink.
- `stipple.vary`: how much lighter ink shrinks a dot; 0 keeps every dot the same size.
- `stipple.shape`: `auto` draws a dot no wider than the pen as one touch and a larger one as a filled spiral; `point`, `circle` and `spiral` force one shape.

**Pens.** Every dot is a pen lift. A fine pen with `point` dots plots fastest; spiral-filled dots go over the same spot many times, so use a pen that does not flood the paper.

**Looks:** Faithful (`voronoi.count` 10,000).

### `line`

<img src="images/styles/line.png" width="260" align="right" alt="The still life drawn with the line style">

The points joined into one unbroken line that does not cross itself, a travelling-salesman tour.

**Use it for** single-line portraits, and machines where pen lifts are slow or wear the hardware.

**Settings**
- `tsp.optimize`: shortens the line and removes its crossings; off leaves the quicker, rougher nearest-neighbour line.
- `tsp.closed`: ends the line where it started.
- `voronoi.exponent`: 2 by default, which puts the line's ink where the image is dark; 1 spreads the points as `stipple` does.

**Pens.** Any pen that draws a steady line. With one stroke, the pen never lifts.

**Looks:** Faithful (`voronoi.count` 20,000).

### `dashes`

<img src="images/styles/dashes.png" width="260" align="right" alt="The still life drawn with the dashes style">

A short stroke at each point, laid along the image's edges, like quick pen strokes that follow form.

**Use it for** a lively, hand-drawn texture on subjects with clear edges.

**Settings**
- `dashes.length_mm`: the length of each dash.
- `dashes.follow_edges`: lays the dashes along the edges; off lays every dash at one angle.
- `dashes.angle_deg`: the turn from the edge direction, or the fixed angle when edges are not followed.

**Pens.** Every dash is a pen lift. Fine and medium fineliners work well.

**Looks:** Faithful (`voronoi.count` 8,400).

### `tree`

<img src="images/styles/tree.png" width="260" align="right" alt="The still life drawn with the tree style">

The points joined by the shortest tree, so every point is reached and no loop forms. It looks like branching veins or roots.

**Use it for** organic, branching textures.

**Settings.** The tree has none of its own; `voronoi.exponent` is 2, as for every style that joins its points.

**Pens.** Fine pens. The tree is drawn as long branching strokes, and `order.join` is `retrace`, which draws some short twigs twice to join the strokes, so the pen lifts far less often than there are points.

**Looks:** Faithful (`tone.response` auto and `voronoi.count` 20,000).

### `mesh`

<img src="images/styles/mesh.png" width="260" align="right" alt="The still life drawn with the mesh style">

A triangle mesh over the points (their Delaunay triangulation), dense and dark where the points crowd.

**Use it for** a low-poly, technical look.

**Settings**
- `delaunay.max_edge_mm`: leaves out edges longer than this, such as those across light areas, so white stays white.

**Pens.** A fine pen: the mesh draws a lot of line, and close edges merge under a wide pen. `order.join` is `retrace`, so the mesh is drawn in long strokes that draw some short edges twice.

**Looks:** Fine (`voronoi.count` 7,100). Ink squared makes the defaults hard to beat, so it has no Faithful.

### `cells`

<img src="images/styles/cells.png" width="260" align="right" alt="The still life drawn with the cells style">

The outlines of the points' Voronoi cells: small cells where it is dark, large ones where it is light, like a crackle glaze or a dried riverbed.

**Use it for** a cellular, organic texture.

**Settings**
- `cells.max_edge_mm`: leaves out the wall between two points farther apart than this, 16 mm by default, so the cells at the edge of an inked area do not reach across white paper ([decision](decisions/2026-10-17-cells-edge-limit.md)).
- `voronoi.exponent` is 2, as for every style that joins its points.

**Pens.** A fine pen. Every corner of a cell joins three edges, so the outlines cannot be drawn in fewer pieces than half the corners; `order.join` is `retrace`, which draws some short edges twice to join the pieces and lifts the pen about a seventh as often. Set it to `on` to draw every edge once, with many more lifts.

**Looks:** Faithful (`voronoi.count` 14,000).

### `scribble`

<img src="images/scribble.png" width="260" align="right" alt="The still life drawn with the scribble style">

Loops traced along the same line `line` draws through the points: small and close where the image is dark, wide and loose where it is light, like circular scribble art.

**Use it for** an energetic single-stroke look, and when you want `line`'s single stroke with more darkness in the shadows.

**Settings**
- `scribble.spacing_mm`: how far the loops move along the line in one turn over full ink. Closer loops draw darker.
- `scribble.white_spacing_mm`: the same over white.
- `scribble.radius_mm` and `scribble.white_radius_mm`: the loop size over full ink and over white.
- `scribble.flatten` and `scribble.tilt_deg`: flatten the loops into ellipses and turn their long axis, like a tilted pen.

**Pens.** Loops cross each other all the time, so use a pen that does not bleed or pill the paper where it passes again. The whole drawing is one stroke.

**Looks:** Faithful (`voronoi.count` 20,000).

## Sketch lines

### `sketch`

<img src="images/styles/sketch.png" width="260" align="right" alt="The still life drawn with the sketch style">

Straight strokes chained from dark to dark. From the pen it tries many directions, draws the darkest, lightens the map under the stroke, and carries on, so the strokes pile up where the image is darkest, like a quick pencil sketch.

**Use it for** a loose, gestural look.

**Settings**
- `sketch.max_lines` (density): the most segments to draw.
- `sketch.length_mm`: the length of each segment. Longer gives a looser drawing.
- `sketch.lighten`: the ink removed under each segment. Less gives more, denser strokes.
- `sketch.stop`: the ink left below which no stroke is drawn. Higher ends the drawing sooner and breaks it into more strokes.
- `sketch.candidates`: the directions tried for each segment. More find darker paths and run slower.
- `sketch.seed`: picks other directions.

**Pens.** Fine pens give the pencil look. Strokes cross and retrace dark areas, so a wet pen can soak the paper there.

**Looks:** Faithful (`tone.response` auto and `sketch.max_lines` 17,000) and Fine (`sketch.max_lines` 7,100).

## Other placers

### `halftone`

<img src="images/styles/halftone.png" width="260" align="right" alt="The still life drawn with the halftone style">

Dots on a regular grid, each sized by the ink, like a printed halftone screen.

**Use it for** a print, newspaper or pop-art look.

**Settings**
- `points.spacing_mm` (density): the screen pitch.
- `stipple.size_mm`: the dot diameter over full ink.
- `stipple.vary`: how much lighter ink shrinks a dot.
- `stipple.shape`: as for `stipple`.
- `points.threshold`: the least ink that gets a dot.

**Pens.** Large dots are filled with spirals, so a wider pen fills them in fewer turns and plots faster. Every dot is a pen lift; this is one of the slowest styles to plot.

**Looks:** Balanced only: with the 0.3 mm pen it leaves specks on light gradients at its default spacing and every finer one.

### `dither`

<img src="images/styles/dither.png" width="260" align="right" alt="The still life drawn with the dither style">

Dots where error diffusion puts them: each grid cell gets a dot or none, and the rounding error is passed on to the neighbours, so the dots' density follows the tone.

**Use it for** a crisp, digital texture with true midtones.

**Settings**
- `dither.spacing_mm` (density): the grid cell; at most one dot per cell.
- `dither.serpentine`: scans alternate rows in opposite directions, which breaks up diagonal patterns.

- `render.size`: a dot's width as a share of the spacing. At 1, the default, neighbouring dots touch, so the dots' density alone carries the tone.

**Pens.** Each dot is filled with a spiral, so a wider pen fills it in fewer turns and plots faster. A dot no wider than the pen is one touch of the pen. Every dot is a pen lift.

**Looks:** Fine (`dither.spacing_mm` 0.7). No Quick: the dots' ink alone takes most of half an hour, and the spacing that gets there loses the picture.

### `circles`

<img src="images/styles/circles.png" width="260" align="right" alt="The still life drawn with the circles style">

Circles packed without overlapping, larger and closer where it is darker.

**Use it for** a bubbly, graphic texture.

**Settings**
- `poisson.spacing_mm` (density): the distance between circle centres over full ink. Lighter areas space them further apart.
- `stipple.size_mm`: the circle diameter over full ink.
- `stipple.vary`: how much lighter ink shrinks a circle.
- `stipple.shape`: `circle` by default; `spiral` fills them.
- `poisson.seed`: picks another packing.

**Pens.** Any pen; the circles are hollow, so they take little ink.

**Looks:** Faithful (`poisson.spacing_mm` 2.1).

## Tone only

These styles draw from the tone map alone, with no points.

### `contours`

<img src="images/styles/contours.png" width="260" align="right" alt="The still life drawn with the contours style">

Level lines of the image, like a topographic map: each line follows one tone.

**Use it for** a cartographic look, and subjects with smooth shading such as faces.

**Settings**
- `contours.levels` (density): the number of levels between white and full ink.
- `contours.smooth_mm`: blurs the map before tracing. More gives rounder, calmer lines.

**Pens.** Any pen. The lines are long, so the pen lifts rarely.

**Looks:** Balanced only: its default levels already stack where the tone is steep.

### `flow`

<img src="images/styles/flow.png" width="260" align="right" alt="The still life drawn with the flow style">

Evenly spaced lines that follow the image's edges, closer together where it is darker.

**Use it for** a combed, wood-grain or brushed look that follows form.

**Settings**
- `flow.min_spacing_mm` (density): the line spacing over full ink.
- `flow.max_spacing_mm`: the line spacing over white.
- `flow.smooth_mm`: blurs the map before the edge directions are measured. More gives longer, smoother lines.
- `flow.angle_deg`: the direction lines take where the image is flat.

**Pens.** Any pen; the lines are long.

**Looks:** Balanced only: auto response scored best but turned the picture into flat bands, and no spacing without it scores clearly better than the default.

### `hilbert`

<img src="images/styles/hilbert.png" width="260" align="right" alt="The still life drawn with the hilbert style">

One space-filling curve over the whole page that folds tighter where it is darker. With one pen it is a single unbroken line.

**Use it for** a maze-like, digital texture, and single-stroke plots.

**Settings**
- `hilbert.cell_mm` (density): the smallest fold, used over full ink.
- `hilbert.max_cell_mm`: the largest fold, used over white.

**Pens.** Keep `cell_mm` above the pen width so the finest folds stay open.

**Looks:** Faithful (`hilbert.cell_mm` 0.4) and Quick (0.9 mm).

### `mosaic`

<img src="images/mosaic.png" width="260" align="right" alt="The still life drawn with the mosaic style">

The image cut into patches that follow its shapes (SLIC superpixels), each filled with parallel lines along its own long axis, closer together where it is darker. Dark patches zig-zag in one stroke; light ones stay loose strokes.

**Use it for** a painterly, patchwork hatching where the strokes follow the shapes.

**Settings**
- `mosaic.spacing_mm` (density): the line spacing over full ink. Lighter patches space theirs wider.
- `mosaic.region_mm`: about how wide one patch is.
- `mosaic.compactness`: higher keeps the patches round; lower lets them follow the image's shapes.

**Pens.** Fineliners. Keep `spacing_mm` above the pen width.

**Looks:** Faithful (`mosaic.spacing_mm` 0.3: the tone error falls from 0.044 to 0.017).
