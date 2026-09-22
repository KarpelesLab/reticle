// Immediate, deferred and concurrent assertions, action blocks, property
// and sequence declarations (kept verbatim) and bind.
module dut(input logic clk, rst, req, ack, input logic [3:0] cnt);
    always @(posedge clk) begin
        assert (cnt < 4'd10);
        assert (cnt != 4'd15) else $error("saturated");
        assert (req -> ack) $display("ok"); else $fatal(1, "bad");
        assume (rst == 0) else $warning("reset");
        cover (ack) $info("covered");
        assert #0 (cnt > 0);
        assert final (cnt > 0) else $error("final");
        assume #0 (req) $display("assumed");
        chk: assert (cnt[0]) else begin
            $error("odd");
            $finish;
        end
    end

    a_req_ack: assert property (@(posedge clk) disable iff (rst) req |-> ##[1:3] ack)
        else $error("no ack");
    assert property (@(posedge clk) req |=> ack);
    assume property (@(posedge clk) !(req && ack));
    c_ack: cover property (@(posedge clk) req ##1 ack);
    cover sequence (@(posedge clk) req ##1 ack);
    restrict property (@(posedge clk) cnt < 8);
    a_lbl: assert property (p_stable(cnt)) $display("stable"); else $error("changed");

    property p_stable(x);
        @(posedge clk) $stable(x) or $rose(req);
    endproperty

    sequence s_handshake;
        req ##[1:$] ack;
    endsequence

    default clocking cb @(posedge clk);
    endclocking
endmodule

module checker_mod(input logic clk, input logic [3:0] cnt);
    a: assert property (@(posedge clk) cnt != 4'hf);
endmodule

bind dut checker_mod chk_i (.clk(clk), .cnt(cnt));
bind dut: u0, u1 checker_mod chk_j (.*);
bind top.dut_inst checker_mod #(.P(1)) chk_k (.clk, .cnt);
