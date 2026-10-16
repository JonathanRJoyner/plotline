Section: Output

- **HPGL export:** a drawing can be written as one HPGL file for pen plotters that read that format, with a pen selected for each layer and 40 plotter units to the millimetre unless set otherwise. The Export panel has an HPGL button and the Settings dialog an HPGL section for the units; the command line takes `--hpgl FILE` and `--hpgl-units N`, Python has `Session.hpgl` and `Session.export_hpgl`, and the MCP `export` tool takes the format `hpgl` and `units_per_mm`.
