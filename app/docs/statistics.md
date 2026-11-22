# Statistics

Statistics turns the metadata of your library into a picture of how you shoot:
which cameras and lenses you use, which focal lengths you keep, and how many
of your frames you rate as keepers. Open it from the chart button on the rail,
or View > Statistics. Esc or **Done** closes it, and the editor is as you left
it. Everything is read from Immage's local search index, so nothing leaves your
computer.

## Reading the page

- **Overview**: a headline, a coverage line and six cards: photos, top images,
  gear, favourite focal length, median setup and peak hour. Click a card to
  clear the filters (Photos), keep only top images (Top images), jump to
  Cameras (Gear), filter to your favourite focal range, or filter to your peak
  hour and month.
- **Cameras**: the year chips show how many photos you shot in each year
  (**That year**) or up to it (**Cumulative**); the maker donut and its legend
  show each maker's share; Keeper rate by body ranks bodies by the share of
  their frames that reached your top-images rating; Top bodies shows the best
  frame of each body.
- **Lenses**: a ranking by most used, keeper rate or top images, either per
  exact lens model or grouped by focal range, and a field-of-view fan: each
  wedge is a focal range at its real angle of view on a 35 mm frame, and the
  busier the range, the longer its wedge. Hatched bars come from fewer than 100
  photos and are a hint rather than a measure.

- **Exposure**: five aperture rings (wide open to stopped down), where the
  hole in each ring shrinks as the f-number grows and the outer arc is the
  group's share of photos; six shutter regimes from Freeze to Long as bars with
  their counts; and a seven-step ISO ladder, 100 to 6400+. Each ring, bar and
  step filters the page. The header shows your median aperture, shutter speed
  and ISO, and the insights name your most-used stop, the stop whose frames
  reach your top-images rating most often, and your share of long exposures.
- **Time**: a 24-hour clock with one wedge per hour (hour 0 at the top), its
  length the photo count; hours 05-08 and 16-20 are amber (early and late
  light, by clock hour only, not sun position), 21-04 violet (night), the rest
  cyan, and the legend repeats this with shapes. The centre shows the busiest
  hour, or the one you picked; hover or focus a wedge to see its hour. Beside
  it, Seasonality shows photos per month across all years, with the share that
  reached your top-images rating on the same scale; click a month to filter.
  Times are the camera's local capture time.

- **Places**: a map of where the photos with GPS were taken, with the side
  panel of the selected area. Each bubble is an area; its size follows the
  photo count (the biggest area on screen is the biggest bubble) and its
  label gives the count (1.2k). Areas that overlap at the current zoom are
  merged into one bubble with a dashed outline; click it to zoom in and split
  it. A lone photo is a small green dot. The mode switch at the top left
  sizes bubbles by all photos, by top images only, or draws a Heat map. Click a
  bubble to select its area: it turns amber, a Place chip joins the filter
  bar (and every chart re-aggregates; the map itself keeps showing the other
  areas), and the panel shows its numbers, radius, usual body, lens and focal
  length, when it was shot, its six best frames (click to open) and **Open N
  in grid**. The x clears the area, and the chips at the bottom jump to the
  biggest areas, named by position (Statistics makes no network calls to name
  places). Zoom with the + and - buttons, or Ctrl/Cmd and the wheel or a
  trackpad pinch (a plain wheel scrolls the page); the button with four corners
  fits all photos. Panning and zooming ask for the cells in view, a moment
  after you stop. Tiles are OpenStreetMap's, through the same cached tile
  download as the location editor, loaded only for the part in view and only
  once the section is near the screen; the attribution stays at the bottom
  right. Without a connection the map keeps what was cached, says so, and
  draws the areas on a labelled grid that works the same. A line under the map
  counts the photos without GPS, which are not on it. If no photo has GPS the
  section is a short note; if the filters match none it says so with a way out.
  With the keyboard, Tab enters the bubbles and the arrow keys move to the
  nearest one; Enter selects.

The Top images section is added in a later release. With the keyboard,
Tab enters a chart and the arrow keys move between its marks; Enter or Space
filters.

## Filtering by clicking

Every bar, slice, row, chip and wedge is a filter. Click one to set it, click
it again to clear it, click another in the same chart to replace it, and
Shift- or Cmd/Ctrl-click to add to it. The whole page re-aggregates, except
that each chart ignores its own filter so you can still see the alternatives.
Photo cards never filter: they open the photo. (On Top bodies the card filters
and a small button on the picture opens the frame.)

Active filters are chips in the bar under the tabs; the x removes one, **Clear
all** removes all, and Esc removes the last one added (a second Esc closes the
view). The bar always shows how many photos match and how many of them are top
images. **Open N in grid** shows the matching photos in the grid, best rated
first, as search results are, and opens them like any other photo (edits go to
the photo's own sidecar). It shows at most 20,000 photos; with more matches,
narrow the filter. The Grid button on a maker row opens just that maker's
photos.

## Scope, top images and labels

- **Scope**: Whole library, This folder (the folder open in the Folders panel)
  or a Date range, with presets for this year, last year and the last 12
  months. Filters that cannot exist in the new scope are dropped.
- **Top images**: the stepper sets the rating that counts as "top" (1 to 5
  stars, default 4). It defines the amber parts of the bars, the top counts and
  the keeper rate everywhere, and is remembered. It does not filter by itself.
- **Label**: the five colour dots filter by colour label; select several to
  match any of them.

## Coverage

The line under the headline says what the numbers are based on: how many photos
are analysed, how many are still being indexed, and **Add folders...** (the
same as the Library panel's). While indexing runs the counts update every two
seconds, and the charts refresh once when it finishes; **Pause** and **Resume**
control the indexing. A section's pill (for example "44,602 / 46,880 photos with
camera data") shows how many photos had the data that chart needs. With no
folders added, the page explains what is needed instead of showing empty charts.
Refresh reads the numbers again; they also update about a second after a rating
changes.

Keyboard: every control is focusable; arrow keys move between the marks of a
chart and Enter or Space press one.
