# HashLink shared row grid

`RowGrid.hx` calls the new native export directly, so it does not require an
unpublished wrapper in Ashui. Run it with an isolated copy of the release
`blinc_abi` library and the existing xgpu library. The fixture checks 1x/2x
spanning-cell pixels, authored hit ancestry, rejected duplicate rows and reset.

For sibling checkouts, compile from the ABI repository:

```sh
haxe \
  --class-path ../ashui/haxe --class-path ../hlwindow/haxe \
  --class-path ../hlwgpu/haxe --class-path ../ash/haxelib/ash-simd \
  --class-path ../ash/haxelib/ash-future --class-path fixtures/row-grid \
  -D ash_simd -lib hashlink -lib tink_hxx -w -WDeprecated \
  --macro 'ashui.core.render.UiFramework.register()' \
  -main RowGrid -hl /path/to/isolated/row-grid.hl
```

Place `blinc_abi.hdll`, `xgpu.hdll` and the runtime's native libraries beside
the compiled fixture, then run that file with Ash. Preserve the original
consumer's sources and libraries when making compatibility comparisons.
