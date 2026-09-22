// typedef, enum, packed struct and union forms.
typedef logic [7:0] byte_t;
typedef byte_t word_t[2];
typedef int unsigned uint_t;
typedef logic [3:0][7:0] quad_t;

typedef enum {RED, GREEN, BLUE} colour_t;
typedef enum bit [2:0] {IDLE = 0, RUN = 3'd1, DONE = 3'b111} state_t;
typedef enum logic [7:0] {OP_ADD = 8'h10, OP_SUB, REG[4], IMM[1:3] = 8'h40} opcode_t;
typedef enum shortint {NEG = -1, ZERO, POS} sign_t;

typedef struct packed {
  logic       valid;
  logic [3:0] tag;
  byte_t      data;
} packet_t;

typedef struct {
  int      count;
  string   name;
  packet_t pkts[4];
} bundle_t;

typedef union packed {
  logic [15:0]                          word;
  struct packed {
    logic [7:0] hi, lo;
  } bytes;
} half_t;

typedef struct packed signed {
  logic       s;
  logic [6:0] mag;
} fixed_t;

typedef fwd_t;
typedef fwd_s;
typedef plain_fwd;

module m;
  packet_t p;
  bundle_t b;
  half_t h;
  colour_t c = GREEN;
  opcode_t op;
  quad_t q;
  word_t w;
  fixed_t f;
  typedef logic [1:0] local_t;
  local_t l;
  typedef enum {A, B} inner_e;
  inner_e e;
  assign h.bytes.hi = p.data;
  assign p          = '{valid: 1'b1, tag: 4'd2, data: 8'hff};
  assign q          = '{default: '0};
endmodule
