# Desktop GUI design system

The native GUI uses a compact charcoal and lavender palette. Surfaces remain
opaque and layered so text stays legible when the desktop compositor does not
provide blur. Glass is a visual accent, not a transparency dependency.

## Tokens

| Role | Value |
| --- | --- |
| Window background | `#17151F` |
| Smoked surface | `#211E2C` |
| Elevated surface | `#2B2638` |
| Primary lavender | `#B9A0F8` |
| Soft lavender | `#D0C0FF` |
| Muted lavender | `#8F7BBE` |
| Primary text | `#F1EDF8` |
| Secondary text | `#A9A2B8` |
| Subtle border | `#FFFFFF18` |

Spacing uses 4, 8, 12, 16, 24 and 32 px steps. Controls use an 8 px radius,
cards 12 px, and window surfaces 16 px. Typography follows the system UI font
with reusable 11, 13, 15, 20 and 24 px sizes.

## Components

`gui::design` is the source of truth for the Iced theme, color tokens, spacing,
radii, typography, surfaces, primary and secondary buttons, dropdowns, and
connection indicators. Cards use the smoked surface; elevated controls use the
raised surface. Lavender is reserved for primary actions and selected states.
Connection indicators use green for connected, grey for disconnected, and
yellow for warning states.

The design system does not depend on compositor blur, theme-specific
transparency, or custom font files. Individual screens should use these shared
styles rather than introducing local colors or spacing values.
