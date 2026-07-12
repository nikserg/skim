# Skim — logo assets (concept 3a)

The mark: a violet disc with two rounded "read-lines"; the lower line is shorter
(62% opacity) for rhythm. One accent color, no gradients.

## Colors
- Accent (light):  #6b46f2
- Accent (dark):   #8b6dff
- Ink:             #17171b
- Ink (dark bg):   #0d0d10

## Files
- skim-icon.svg        Round mark, light accent (use on light/neutral surfaces)
- skim-icon-dark.svg   Round mark for dark theme (#8b6dff on dark ink)
- skim-tile.svg        Rounded-square app tile, 256×256, padded — source for the Windows .ico
- skim-glyph.svg       Lines only, transparent bg, fill=currentColor
- skim-lockup.svg      Mark + "Skim" wordmark (Hanken Grotesk 700)

## Windows .ico
Rasterize skim-tile.svg to PNGs at 16, 32, 48, 64, 128, 256 and pack into app.ico, e.g.:

    for s in 16 32 48 64 128 256; do
      rsvg-convert -w $s -h $s skim-tile.svg -o icon_$s.png
    done
    magick icon_16.png icon_32.png icon_48.png icon_64.png icon_128.png icon_256.png app.ico

Wordmark font: Hanken Grotesk (Google Fonts / OFL).
