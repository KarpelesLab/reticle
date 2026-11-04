# Where cells may go.
create_pblock pb_sub
add_cells_to_pblock [get_pblocks pb_sub] [get_cells u_sub]
resize_pblock [get_pblocks pb_sub] -add {SLICE_X0Y0:SLICE_X9Y9}
create_pblock pb_regs
add_cells_to_pblock pb_regs [get_cells {r_reg[*]}]
add_cells_to_pblock pb_regs [get_cells m_reg]
resize_pblock pb_regs -add SLICE_X20Y20:SLICE_X25Y29
create_pblock pb_mixed
add_cells_to_pblock [get_pblocks pb_mixed] [get_cells {t_reg[*]}]
resize_pblock [get_pblocks pb_mixed] -add {SLICE_X30Y30:SLICE_X39Y39 RAMB18_X0Y0:RAMB18_X0Y3 DSP48_X0Y0:DSP48_X0Y1}
create_pblock pb_two
add_cells_to_pblock [get_pblocks pb_two] [get_cells {q_reg[*]}]
resize_pblock [get_pblocks pb_two] -add SLICE_X40Y40:SLICE_X41Y41
resize_pblock [get_pblocks pb_two] -add SLICE_X44Y44:SLICE_X45Y45
create_pblock pb_clockregion
add_cells_to_pblock [get_pblocks pb_clockregion] [get_cells {div_reg[*]}]
resize_pblock [get_pblocks pb_clockregion] -add CLOCKREGION_X0Y0:CLOCKREGION_X0Y0
set_property LOC SLICE_X50Y50 [get_cells clk_div_reg]
set_property BEL AFF [get_cells clk_div_reg]
set_property LOC SLICE_X51Y51 [get_cells u_sub/q_reg]
set_property BEL SLICEL.BFF [get_cells u_sub/q_reg]
set_property ASYNC_REG TRUE [get_cells u_sub/s1_reg]
set_property KEEP_HIERARCHY TRUE [get_cells u_sub]
