-- Locally static expressions of every scalar class.
package static_pkg is
  constant A : integer := 2 + 3 * 4;
  constant B : integer := (2 + 3) * 4;
  constant C : integer := 17 mod 5;
  constant D : integer := -17 mod 5;
  constant E : integer := 17 rem 5;
  constant F : integer := 2 ** 10;
  constant G : integer := abs (-7);
  constant H : integer := 16#FF#;
  constant I : integer := 2#1010#;
  constant J : integer := 1_000_000;

  constant K : real := 1.5 + 2.5;
  constant L : real := 3.0 / 2.0;
  constant M : real := 1.0e3;
  constant N : real := 2.0 ** 3;

  constant O : boolean := A > B;
  constant P : boolean := A = 14 and B = 20;
  constant Q : boolean := not false;

  constant R : bit_vector(3 downto 0) := "1010";
  constant S : bit_vector(3 downto 0) := R and "1100";
  constant T : bit_vector(3 downto 0) := not R;
  constant U : bit_vector(7 downto 0) := R & "0101";
  constant V : bit                    := R(3);

  constant W : time := 10 ns;
  constant X : time := W * 3;
  constant Y : string := "ab" & "cd";
end package static_pkg;
