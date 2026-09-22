package counter_pkg is
  type counter_t is protected
    procedure increment(by : in natural := 1);
    impure function value return natural;
    procedure reset;
  end protected counter_t;
end package counter_pkg;

package body counter_pkg is
  type counter_t is protected body
    variable count : natural := 0;

    procedure increment(by : in natural := 1) is
    begin
      count := count + by;
    end procedure increment;

    impure function value return natural is
    begin
      return count;
    end function value;

    procedure reset is
    begin
      count := 0;
    end procedure;
  end protected body counter_t;

  shared variable shared_counter : counter_t;
end package body counter_pkg;
