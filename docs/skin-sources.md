# Skin palette sources

The four original skins keep their original background. The ten additions use colored
backgrounds and 7–8 gradient stops with deeper shadows, saturated midtones, and highlights.
Source anchors below retain the original hex values. Backgrounds, shadow stops, bridge
colors, and peaks are adapted for a luminous music visualizer rather than copied as UI themes.

| Skin | Background | Source palette | Source anchors |
| --- | --- | --- | --- |
| Aurora | `#18234b` | uiGradients / Lunada | `#5433ff` → `#20bdff` → `#a5fecb` |
| Sunset | `#482039` | uiGradients / Instagram | `#833ab4` → `#fd1d1d` → `#fcb045` |
| Lagoon | `#124449` | uiGradients / Aqua Marine | `#1a2980` → `#26d0ce` |
| Candy | `#3d254d` | uiGradients / JShine | `#12c2e9` → `#c471ed` → `#f64f59` |
| Ember | `#461c24` | uiGradients / Flare | `#f12711` → `#f5af19` |
| Citrine | `#303b19` | uiGradients / Harmonic Energy | `#16a085` → `#f4d03f` |
| Dracula | `#282a36` | Dracula / Classic | `#8be9fd` → `#bd93f9` → `#ff79c6` |
| Mocha | `#1e1e2e` | Catppuccin / Mocha | `#89b4fa` → `#cba6f7` → `#f5c2e7` → `#fab387` |
| Ocean | `#091e3a` | uiGradients / Omolon | `#091e3a` → `#2f80ed` → `#2d9ee0` |
| Neon | `#12343b` | uiGradients / Ibtesam | `#00f5a0` → `#00d9f5` |

Sources:

- [uiGradients palette collection](https://github.com/Ghosh/uiGradients/blob/master/gradients.json)
  by the uiGradients contributors ([MIT license](https://github.com/Ghosh/uiGradients/blob/master/LICENSE.md)).
- [Dracula Classic specification](https://draculatheme.com/spec)
  by Zeno Rocha and contributors ([MIT license](https://github.com/dracula/dracula-theme/blob/master/LICENSE)).
- [Catppuccin Mocha palette](https://catppuccin.com/palette/)
  by Catppuccin ([MIT license](https://github.com/catppuccin/palette/blob/main/LICENSE)).

Press **S** to cycle all 14 skins. Desktop and offline renders accept `--skin aurora`,
`--skin sunset`, etc.; the WebAssembly page accepts `?skin=aurora` in its URL.
The JavaScript prototype also includes all 14 as swatches.
