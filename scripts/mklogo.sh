#!/bin/sh
# mklogo.sh — generate an 80x80 PPM boot logo for kexec-menu
#
# The logo is a right-pointing chevron (">") rendered as PPM (P3).
# Suitable for the kernel's CLUT224 logo system (drivers/video/logo/).
#
# Colors default to gruvbox-dark but can be overridden via environment
# variables using "R G B" strings (0-255 per component):
#
#   LOGO_BG      background  (default: "29 32 33"    — gruvbox bg0_h)
#   LOGO_FG      foreground  (default: "213 196 161" — gruvbox fg2)
#   LOGO_ACCENT  accent band (default: "131 165 152" — gruvbox aqua)
#
# Usage:
#   ./scripts/mklogo.sh > logo.ppm
#   LOGO_BG="0 0 0" LOGO_FG="255 255 255" ./scripts/mklogo.sh > logo.ppm
#
# Dependencies: awk (POSIX awk, gawk, or mawk)

set -eu

BG="${LOGO_BG:-29 32 33}"
FG="${LOGO_FG:-213 196 161}"
ACCENT="${LOGO_ACCENT:-131 165 152}"

awk -v bg="$BG" -v fg="$FG" -v accent="$ACCENT" '
  function dist_to_seg(px, py, x1, y1, x2, y2,    dx, dy, len2, t, cx, cy) {
    dx = x2 - x1
    dy = y2 - y1
    len2 = dx*dx + dy*dy
    if (len2 == 0) return sqrt((px-x1)^2 + (py-y1)^2)
    t = ((px-x1)*dx + (py-y1)*dy) / len2
    if (t < 0) t = 0
    if (t > 1) t = 1
    cx = x1 + t*dx
    cy = y1 + t*dy
    return sqrt((px-cx)^2 + (py-cy)^2)
  }

  BEGIN {
    W = 80; H = 80

    split(bg, bgc, " ")
    split(fg, fgc, " ")
    split(accent, acc, " ")

    # Chevron geometry: right-pointing ">"
    # Upper arm: top-left to tip
    ax1 = 20; ay1 = 15; ax2 = 60; ay2 = 40
    # Lower arm: bottom-left to tip
    bx1 = 20; by1 = 65; bx2 = 60; by2 = 40

    ht = 3.5   # half-thickness of chevron arms
    aa = 1.5   # anti-alias / accent band width

    printf "P3\n%d %d\n255\n", W, H

    for (y = 0; y < H; y++) {
      for (x = 0; x < W; x++) {
        d1 = dist_to_seg(x, y, ax1, ay1, ax2, ay2)
        d2 = dist_to_seg(x, y, bx1, by1, bx2, by2)
        d = (d1 < d2) ? d1 : d2

        if (d <= ht) {
          printf "%d %d %d\n", fgc[1], fgc[2], fgc[3]
        } else if (d <= ht + aa) {
          printf "%d %d %d\n", acc[1], acc[2], acc[3]
        } else {
          printf "%d %d %d\n", bgc[1], bgc[2], bgc[3]
        }
      }
    }
  }
' /dev/null
