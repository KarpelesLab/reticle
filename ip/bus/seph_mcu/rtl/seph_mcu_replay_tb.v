// seph_mcu against a real secure element's radio and USB configuration,
// replayed byte for byte from a capture.
//
// The script below is generated from `visgrok`'s decode of a real Nano X
// (`capture-20261009-125946.seph.log`, SE boot 1, SE 2.6.0 and MCU 2.39.0):
// every packet the SE sent from SESSION_START until the MCU had answered
// its fifteenth BLE command, `aci_gap_set_discoverable`. General statuses and
// REQUEST_STATUS are left out of it: this test is about configuration. The
// SE's USB_CONFIG packets (`4F`: disconnect, address 0, connect, both halves
// of endpoint 0) are kept, in the turns where the SE sent them.
//
// The model SE sends one turn and ends it with a status, and answers any
// other event -- a ticker -- with a status and nothing more, as an SE with
// nothing to say does. Each BLE_RECV_EVENT the block sends must be
// **byte for byte** the one the real MCU sent for that command, return
// parameters included, and then the next turn goes out.
//
// What it checks that `seph_mcu_tb.v` does not: all fifteen commands of a
// real configuration, in order, including the three characteristics whose
// handles depend on their properties. That is the case that was wrong --
// the block counted two handles per characteristic, and the real radio
// answered 0d, 10, 12 where the block would have said 0d, 0f, 11 -- and
// this test fails on it.
//
// What it would not catch: anything after configuration (a connection, an
// APDU over BLE or USB), and an MCU that answers USB_CONFIG. The real MCU
// answered `connect` with USB_EVENT reset because a host was plugged in;
// with no host there is nothing to answer, and this block sends nothing.
`timescale 1ns / 1ps
module seph_mcu_replay_tb;
    localparam integer MS = 4;

    reg clk = 1'b0;
    always #5 clk = ~clk;
    reg rst_n = 1'b0;

    reg        start = 1'b0;
    reg  [7:0] rx_data = 8'd0;
    reg        rx_valid = 1'b0;
    wire [7:0] tx_data;
    wire       tx_valid;
    wire       active, started;
    wire [15:0] rx_packets, tx_events;
    wire [7:0] last_rx_tag;
    wire [31:0] ms;

    seph_mcu #(.MS_CYCLES(MS), .TURN_CYCLES(6)) dut (
        .clk(clk), .rst_n(rst_n), .start(start), .stop(1'b0),
        .rx_data(rx_data), .rx_valid(rx_valid),
        .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(1'b1),
        .buttons(2'b00),
        .usb_plugged(1'b0), .usb_on(),
        .inj_start(1'b0), .inj_data(8'd0), .inj_valid(1'b0), .inj_commit(1'b0),
        .inj_busy(), .inj_bad(), .active(active), .started(started),
        .rx_packets(rx_packets), .tx_events(tx_events),
        .last_rx_tag(last_rx_tag), .ms(ms));

    localparam integer TURNS = 15;
    reg [7:0] se_b [0:315];
    integer   se_e [0:14];
    reg [7:0] ans_b [0:163];
    integer   ans_e [0:14];
    initial begin
        se_b[0] = 8'h4e; se_b[1] = 8'h00; se_b[2] = 8'h02; se_b[3] = 8'h00; se_b[4] = 8'h64; se_b[5] = 8'h4e; se_b[6] = 8'h00; se_b[7] = 8'h02;
        se_b[8] = 8'h00; se_b[9] = 8'h64; se_b[10] = 8'h31; se_b[11] = 8'h00; se_b[12] = 8'h01; se_b[13] = 8'h01; se_b[14] = 8'h38; se_b[15] = 8'h00;
        se_b[16] = 8'h02; se_b[17] = 8'h0c; se_b[18] = 8'h03; se_b[19] = 8'h4f; se_b[20] = 8'h00; se_b[21] = 8'h01; se_b[22] = 8'h02; se_b[23] = 8'h4f;
        se_b[24] = 8'h00; se_b[25] = 8'h02; se_b[26] = 8'h03; se_b[27] = 8'h00; se_b[28] = 8'h4f; se_b[29] = 8'h00; se_b[30] = 8'h01; se_b[31] = 8'h01;
        se_b[32] = 8'h4e; se_b[33] = 8'h00; se_b[34] = 8'h02; se_b[35] = 8'h00; se_b[36] = 8'h64; se_b[37] = 8'h4f; se_b[38] = 8'h00; se_b[39] = 8'h05;
        se_b[40] = 8'h04; se_b[41] = 8'h01; se_b[42] = 8'h00; se_b[43] = 8'h01; se_b[44] = 8'h40; se_b[45] = 8'h4f; se_b[46] = 8'h00; se_b[47] = 8'h05;
        se_b[48] = 8'h04; se_b[49] = 8'h01; se_b[50] = 8'h80; se_b[51] = 8'h01; se_b[52] = 8'h40; se_b[53] = 8'h38; se_b[54] = 8'h00; se_b[55] = 8'h0a;
        se_b[56] = 8'hfc; se_b[57] = 8'h0c; se_b[58] = 8'h2e; se_b[59] = 8'h06; se_b[60] = 8'hb3; se_b[61] = 8'h75; se_b[62] = 8'h0c; se_b[63] = 8'hbe;
        se_b[64] = 8'hb2; se_b[65] = 8'hc5; se_b[66] = 8'h38; se_b[67] = 8'h00; se_b[68] = 8'h02; se_b[69] = 8'hfd; se_b[70] = 8'h01; se_b[71] = 8'h38;
        se_b[72] = 8'h00; se_b[73] = 8'h05; se_b[74] = 8'hfc; se_b[75] = 8'h8a; se_b[76] = 8'h01; se_b[77] = 8'h00; se_b[78] = 8'h14; se_b[79] = 8'h38;
        se_b[80] = 8'h00; se_b[81] = 8'h03; se_b[82] = 8'hfc; se_b[83] = 8'h85; se_b[84] = 8'h01; se_b[85] = 8'h38; se_b[86] = 8'h00; se_b[87] = 8'h0e;
        se_b[88] = 8'hfc; se_b[89] = 8'h86; se_b[90] = 8'h01; se_b[91] = 8'h01; se_b[92] = 8'h01; se_b[93] = 8'h00; se_b[94] = 8'h08; se_b[95] = 8'h10;
        se_b[96] = 8'h01; se_b[97] = 8'h00; se_b[98] = 8'h00; se_b[99] = 8'h00; se_b[100] = 8'h00; se_b[101] = 8'h01; se_b[102] = 8'h38; se_b[103] = 8'h00;
        se_b[104] = 8'h15; se_b[105] = 8'hfd; se_b[106] = 8'h02; se_b[107] = 8'h02; se_b[108] = 8'h72; se_b[109] = 8'h65; se_b[110] = 8'h67; se_b[111] = 8'h64;
        se_b[112] = 8'h65; se_b[113] = 8'h4c; se_b[114] = 8'h00; se_b[115] = 8'h00; se_b[116] = 8'h04; se_b[117] = 8'h00; se_b[118] = 8'h97; se_b[119] = 8'h2c;
        se_b[120] = 8'h00; se_b[121] = 8'h34; se_b[122] = 8'hd6; se_b[123] = 8'h13; se_b[124] = 8'h01; se_b[125] = 8'h09; se_b[126] = 8'h38; se_b[127] = 8'h00;
        se_b[128] = 8'h1c; se_b[129] = 8'hfd; se_b[130] = 8'h04; se_b[131] = 8'h0c; se_b[132] = 8'h00; se_b[133] = 8'h02; se_b[134] = 8'h72; se_b[135] = 8'h65;
        se_b[136] = 8'h67; se_b[137] = 8'h64; se_b[138] = 8'h65; se_b[139] = 8'h4c; se_b[140] = 8'h01; se_b[141] = 8'h00; se_b[142] = 8'h04; se_b[143] = 8'h00;
        se_b[144] = 8'h97; se_b[145] = 8'h2c; se_b[146] = 8'h00; se_b[147] = 8'h34; se_b[148] = 8'hd6; se_b[149] = 8'h13; se_b[150] = 8'h9c; se_b[151] = 8'h00;
        se_b[152] = 8'h10; se_b[153] = 8'h08; se_b[154] = 8'h00; se_b[155] = 8'h10; se_b[156] = 8'h01; se_b[157] = 8'h38; se_b[158] = 8'h00; se_b[159] = 8'h1c;
        se_b[160] = 8'hfd; se_b[161] = 8'h04; se_b[162] = 8'h0c; se_b[163] = 8'h00; se_b[164] = 8'h02; se_b[165] = 8'h72; se_b[166] = 8'h65; se_b[167] = 8'h67;
        se_b[168] = 8'h64; se_b[169] = 8'h65; se_b[170] = 8'h4c; se_b[171] = 8'h02; se_b[172] = 8'h00; se_b[173] = 8'h04; se_b[174] = 8'h00; se_b[175] = 8'h97;
        se_b[176] = 8'h2c; se_b[177] = 8'h00; se_b[178] = 8'h34; se_b[179] = 8'hd6; se_b[180] = 8'h13; se_b[181] = 8'h9c; se_b[182] = 8'h00; se_b[183] = 8'h08;
        se_b[184] = 8'h08; se_b[185] = 8'h02; se_b[186] = 8'h10; se_b[187] = 8'h01; se_b[188] = 8'h38; se_b[189] = 8'h00; se_b[190] = 8'h1c; se_b[191] = 8'hfd;
        se_b[192] = 8'h04; se_b[193] = 8'h0c; se_b[194] = 8'h00; se_b[195] = 8'h02; se_b[196] = 8'h72; se_b[197] = 8'h65; se_b[198] = 8'h67; se_b[199] = 8'h64;
        se_b[200] = 8'h65; se_b[201] = 8'h4c; se_b[202] = 8'h03; se_b[203] = 8'h00; se_b[204] = 8'h04; se_b[205] = 8'h00; se_b[206] = 8'h97; se_b[207] = 8'h2c;
        se_b[208] = 8'h00; se_b[209] = 8'h34; se_b[210] = 8'hd6; se_b[211] = 8'h13; se_b[212] = 8'h9c; se_b[213] = 8'h00; se_b[214] = 8'h04; se_b[215] = 8'h08;
        se_b[216] = 8'h01; se_b[217] = 8'h10; se_b[218] = 8'h01; se_b[219] = 8'h38; se_b[220] = 8'h00; se_b[221] = 8'h04; se_b[222] = 8'hfc; se_b[223] = 8'h0f;
        se_b[224] = 8'h01; se_b[225] = 8'h07; se_b[226] = 8'h38; se_b[227] = 8'h00; se_b[228] = 8'h0c; se_b[229] = 8'hfc; se_b[230] = 8'h8e; se_b[231] = 8'h09;
        se_b[232] = 8'h02; se_b[233] = 8'h01; se_b[234] = 8'h06; se_b[235] = 8'h05; se_b[236] = 8'h09; se_b[237] = 8'h41; se_b[238] = 8'h39; se_b[239] = 8'h46;
        se_b[240] = 8'h30; se_b[241] = 8'h38; se_b[242] = 8'h00; se_b[243] = 8'h22; se_b[244] = 8'h20; se_b[245] = 8'h09; se_b[246] = 8'h18; se_b[247] = 8'h11;
        se_b[248] = 8'h06; se_b[249] = 8'h72; se_b[250] = 8'h65; se_b[251] = 8'h67; se_b[252] = 8'h64; se_b[253] = 8'h65; se_b[254] = 8'h4c; se_b[255] = 8'h00;
        se_b[256] = 8'h00; se_b[257] = 8'h04; se_b[258] = 8'h00; se_b[259] = 8'h97; se_b[260] = 8'h2c; se_b[261] = 8'h00; se_b[262] = 8'h34; se_b[263] = 8'hd6;
        se_b[264] = 8'h13; se_b[265] = 8'h05; se_b[266] = 8'h12; se_b[267] = 8'h0c; se_b[268] = 8'h00; se_b[269] = 8'h18; se_b[270] = 8'h00; se_b[271] = 8'h00;
        se_b[272] = 8'h00; se_b[273] = 8'h00; se_b[274] = 8'h00; se_b[275] = 8'h00; se_b[276] = 8'h00; se_b[277] = 8'h00; se_b[278] = 8'h38; se_b[279] = 8'h00;
        se_b[280] = 8'h0c; se_b[281] = 8'hfd; se_b[282] = 8'h06; se_b[283] = 8'h05; se_b[284] = 8'h00; se_b[285] = 8'h06; se_b[286] = 8'h00; se_b[287] = 8'h00;
        se_b[288] = 8'h04; se_b[289] = 8'h41; se_b[290] = 8'h39; se_b[291] = 8'h46; se_b[292] = 8'h30; se_b[293] = 8'h38; se_b[294] = 8'h00; se_b[295] = 8'h14;
        se_b[296] = 8'hfc; se_b[297] = 8'h83; se_b[298] = 8'h00; se_b[299] = 8'h30; se_b[300] = 8'h00; se_b[301] = 8'h60; se_b[302] = 8'h00; se_b[303] = 8'h01;
        se_b[304] = 8'h00; se_b[305] = 8'h05; se_b[306] = 8'h09; se_b[307] = 8'h41; se_b[308] = 8'h39; se_b[309] = 8'h46; se_b[310] = 8'h30; se_b[311] = 8'h00;
        se_b[312] = 8'h00; se_b[313] = 8'h00; se_b[314] = 8'h00; se_b[315] = 8'h00;
        ans_b[0] = 8'h18; ans_b[1] = 8'h00; ans_b[2] = 8'h07; ans_b[3] = 8'h04; ans_b[4] = 8'h0e; ans_b[5] = 8'h04; ans_b[6] = 8'h01; ans_b[7] = 8'h03;
        ans_b[8] = 8'h0c; ans_b[9] = 8'h00; ans_b[10] = 8'h18; ans_b[11] = 8'h00; ans_b[12] = 8'h07; ans_b[13] = 8'h04; ans_b[14] = 8'h0e; ans_b[15] = 8'h04;
        ans_b[16] = 8'h01; ans_b[17] = 8'h0c; ans_b[18] = 8'hfc; ans_b[19] = 8'h00; ans_b[20] = 8'h18; ans_b[21] = 8'h00; ans_b[22] = 8'h07; ans_b[23] = 8'h04;
        ans_b[24] = 8'h0e; ans_b[25] = 8'h04; ans_b[26] = 8'h01; ans_b[27] = 8'h01; ans_b[28] = 8'hfd; ans_b[29] = 8'h00; ans_b[30] = 8'h18; ans_b[31] = 8'h00;
        ans_b[32] = 8'h0d; ans_b[33] = 8'h04; ans_b[34] = 8'h0e; ans_b[35] = 8'h0a; ans_b[36] = 8'h01; ans_b[37] = 8'h8a; ans_b[38] = 8'hfc; ans_b[39] = 8'h00;
        ans_b[40] = 8'h05; ans_b[41] = 8'h00; ans_b[42] = 8'h06; ans_b[43] = 8'h00; ans_b[44] = 8'h08; ans_b[45] = 8'h00; ans_b[46] = 8'h18; ans_b[47] = 8'h00;
        ans_b[48] = 8'h07; ans_b[49] = 8'h04; ans_b[50] = 8'h0e; ans_b[51] = 8'h04; ans_b[52] = 8'h01; ans_b[53] = 8'h85; ans_b[54] = 8'hfc; ans_b[55] = 8'h00;
        ans_b[56] = 8'h18; ans_b[57] = 8'h00; ans_b[58] = 8'h07; ans_b[59] = 8'h04; ans_b[60] = 8'h0e; ans_b[61] = 8'h04; ans_b[62] = 8'h01; ans_b[63] = 8'h86;
        ans_b[64] = 8'hfc; ans_b[65] = 8'h00; ans_b[66] = 8'h18; ans_b[67] = 8'h00; ans_b[68] = 8'h09; ans_b[69] = 8'h04; ans_b[70] = 8'h0e; ans_b[71] = 8'h06;
        ans_b[72] = 8'h01; ans_b[73] = 8'h02; ans_b[74] = 8'hfd; ans_b[75] = 8'h00; ans_b[76] = 8'h0c; ans_b[77] = 8'h00; ans_b[78] = 8'h18; ans_b[79] = 8'h00;
        ans_b[80] = 8'h09; ans_b[81] = 8'h04; ans_b[82] = 8'h0e; ans_b[83] = 8'h06; ans_b[84] = 8'h01; ans_b[85] = 8'h04; ans_b[86] = 8'hfd; ans_b[87] = 8'h00;
        ans_b[88] = 8'h0d; ans_b[89] = 8'h00; ans_b[90] = 8'h18; ans_b[91] = 8'h00; ans_b[92] = 8'h09; ans_b[93] = 8'h04; ans_b[94] = 8'h0e; ans_b[95] = 8'h06;
        ans_b[96] = 8'h01; ans_b[97] = 8'h04; ans_b[98] = 8'hfd; ans_b[99] = 8'h00; ans_b[100] = 8'h10; ans_b[101] = 8'h00; ans_b[102] = 8'h18; ans_b[103] = 8'h00;
        ans_b[104] = 8'h09; ans_b[105] = 8'h04; ans_b[106] = 8'h0e; ans_b[107] = 8'h06; ans_b[108] = 8'h01; ans_b[109] = 8'h04; ans_b[110] = 8'hfd; ans_b[111] = 8'h00;
        ans_b[112] = 8'h12; ans_b[113] = 8'h00; ans_b[114] = 8'h18; ans_b[115] = 8'h00; ans_b[116] = 8'h07; ans_b[117] = 8'h04; ans_b[118] = 8'h0e; ans_b[119] = 8'h04;
        ans_b[120] = 8'h01; ans_b[121] = 8'h0f; ans_b[122] = 8'hfc; ans_b[123] = 8'h00; ans_b[124] = 8'h18; ans_b[125] = 8'h00; ans_b[126] = 8'h07; ans_b[127] = 8'h04;
        ans_b[128] = 8'h0e; ans_b[129] = 8'h04; ans_b[130] = 8'h01; ans_b[131] = 8'h8e; ans_b[132] = 8'hfc; ans_b[133] = 8'h00; ans_b[134] = 8'h18; ans_b[135] = 8'h00;
        ans_b[136] = 8'h07; ans_b[137] = 8'h04; ans_b[138] = 8'h0e; ans_b[139] = 8'h04; ans_b[140] = 8'h01; ans_b[141] = 8'h09; ans_b[142] = 8'h20; ans_b[143] = 8'h00;
        ans_b[144] = 8'h18; ans_b[145] = 8'h00; ans_b[146] = 8'h07; ans_b[147] = 8'h04; ans_b[148] = 8'h0e; ans_b[149] = 8'h04; ans_b[150] = 8'h01; ans_b[151] = 8'h06;
        ans_b[152] = 8'hfd; ans_b[153] = 8'h00; ans_b[154] = 8'h18; ans_b[155] = 8'h00; ans_b[156] = 8'h07; ans_b[157] = 8'h04; ans_b[158] = 8'h0e; ans_b[159] = 8'h04;
        ans_b[160] = 8'h01; ans_b[161] = 8'h83; ans_b[162] = 8'hfc; ans_b[163] = 8'h00;
        se_e[0] = 53; se_e[1] = 66; se_e[2] = 71; se_e[3] = 79; se_e[4] = 85; se_e[5] = 102; se_e[6] = 126; se_e[7] = 157; se_e[8] = 188; se_e[9] = 219; se_e[10] = 226; se_e[11] = 241; se_e[12] = 278; se_e[13] = 293; se_e[14] = 316;
        ans_e[0] = 10; ans_e[1] = 20; ans_e[2] = 30; ans_e[3] = 46; ans_e[4] = 56; ans_e[5] = 66; ans_e[6] = 78; ans_e[7] = 90; ans_e[8] = 102; ans_e[9] = 114; ans_e[10] = 124; ans_e[11] = 134; ans_e[12] = 144; ans_e[13] = 154; ans_e[14] = 164;
    end

    // What the MCU sends, queued as it goes.
    reg [7:0] q [0:4095];
    integer   q_w = 0, q_r = 0;
    always @(posedge clk)
        if (tx_valid) begin
            q[q_w % 4096] = tx_data;
            q_w = q_w + 1;
        end

    reg [7:0] b;
    integer waited;
    task mcu_byte;
        begin
            waited = 0;
            while (q_r >= q_w) begin
                @(posedge clk);
                waited = waited + 1;
                if (waited > 200000) begin
                    $display("FAIL: the MCU stopped sending (turn %0d)", k);
                    $finish;
                end
            end
            b   = q[q_r % 4096];
            q_r = q_r + 1;
        end
    endtask

    task se_byte;
        input [7:0] v;
        begin
            rx_data  = v;
            rx_valid = 1'b1;
            @(posedge clk);
            rx_valid = 1'b0;
            repeat (3) @(posedge clk);
        end
    endtask

    task se_status;
        begin
            se_byte(8'h60); se_byte(8'h00); se_byte(8'h02);
            se_byte(8'h00); se_byte(8'h00);
        end
    endtask

    integer k, i, n, len, others;
    reg [7:0] tag;
    reg [7:0] ev [0:255];

    initial begin
        repeat (4) @(posedge clk);
        rst_n = 1'b1;
        @(posedge clk);
        start = 1'b1;
        @(posedge clk);
        start = 1'b0;

        // SESSION_START, whole; `seph_mcu_tb.v` checks its bytes.
        mcu_byte; tag = b;
        mcu_byte; len = b; mcu_byte; len = len * 256 + b;
        for (i = 0; i < len; i = i + 1) mcu_byte;
        if (tag !== 8'h01) begin
            $display("FAIL: the first event was %02x, not SESSION_START", tag);
            $finish;
        end

        others = 0;
        for (k = 0; k < TURNS; k = k + 1) begin
            for (i = (k == 0) ? 0 : se_e[k - 1]; i < se_e[k]; i = i + 1)
                se_byte(se_b[i]);
            se_status;
            // Events until the answer; anything else gets a bare status.
            tag = 8'h00;
            while (tag !== 8'h18) begin
                mcu_byte; tag = b; ev[0] = b;
                mcu_byte; ev[1] = b; mcu_byte; ev[2] = b;
                len = ev[1] * 256 + ev[2];
                for (i = 0; i < len; i = i + 1) begin
                    mcu_byte; ev[3 + i] = b;
                end
                if (tag !== 8'h18) begin
                    if (tag !== 8'h0e) begin
                        $display("FAIL: turn %0d: the MCU sent tag %02x, wanted a BLE answer or a ticker",
                                 k, tag);
                        $finish;
                    end
                    others = others + 1;
                    if (others > 400) begin
                        $display("FAIL: turn %0d: no BLE answer after %0d tickers", k, others);
                        $finish;
                    end
                    se_status;
                end
            end
            n = ans_e[k] - ((k == 0) ? 0 : ans_e[k - 1]);
            if (len + 3 != n) begin
                $display("FAIL: turn %0d: the BLE answer is %0d bytes, the real MCU's %0d",
                         k, len + 3, n);
                $finish;
            end
            for (i = 0; i < n; i = i + 1)
                if (ev[i] !== ans_b[((k == 0) ? 0 : ans_e[k - 1]) + i]) begin
                    $display("FAIL: turn %0d (opcode %02x%02x): byte %0d is %02x, the real MCU sent %02x",
                             k, ev[8], ev[7], i, ev[i],
                             ans_b[((k == 0) ? 0 : ans_e[k - 1]) + i]);
                    $finish;
                end
        end
        $display("PASS: %0d BLE commands of a real configuration answered byte for byte as the real MCU did, USB_CONFIG read without an answer, %0d ticker(s) between",
                 TURNS, others);
        $finish;
    end
endmodule
