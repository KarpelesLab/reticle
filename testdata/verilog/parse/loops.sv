// Loops: for with declarations and multiple init/step, while, do-while,
// repeat, forever, foreach with skipped dimensions, break and continue.
module loops;
    int arr [4][8];
    int q [$];
    int total, i, j;
    logic clk;

    initial begin
        for (i = 0; i < 4; i = i + 1) total += i;
        for (int k = 0; k < 4; k++) total += k;
        for (int k = 0, m = 8; k < m; k++, m--) total += k * m;
        for (i = 0, j = 10; i < j; i++, j--) ;
        for (int k = 0, byte b = 0; k < 4; k++) begin
            b += 8'd1;
        end
        for (;;) begin
            if (total > 100) break;
            total++;
        end
        for (i = 0; ; i++) if (i == 3) break;
        for (var int v = 0; v < 2; v += 1) begin end

        while (total > 0) total--;
        while (1) begin
            if (total++ == 5) break;
            else continue;
        end

        do total++; while (total < 10);
        do begin
            total--;
        end while (total > 0);

        repeat (3) total++;
        repeat (arr[0][0]) begin
            @(posedge clk);
        end

        foreach (arr[a, b]) arr[a][b] = a * b;
        foreach (arr[a]) arr[a][0] = a;
        foreach (arr[, b]) arr[0][b] = b;
        foreach (q[k]) begin
            if (q[k] == 0) continue;
            total += q[k];
        end
    end

    initial forever #5 clk = ~clk;

    initial begin
        forever begin
            @(posedge clk);
            if (total == 0) break;
        end
    end
endmodule
