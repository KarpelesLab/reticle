-- `ieee.math_real`: the constants are in the bundled source, the
-- functions are native over `f64`, and both fold at analysis time.
library ieee;
use ieee.math_real.all;

package math_real_static is

  constant two      : real := sqrt(4.0);
  constant e_back   : real := log(math_e);
  constant ten      : real := log10(1.0e10);
  constant bits     : real := log2(256.0);
  constant base3    : real := log(27.0, 3.0);
  constant rounded  : integer := integer(round(2.5));
  constant floored  : integer := integer(floor(-2.5));
  constant ceiled   : integer := integer(ceil(-2.5));
  constant truncd   : integer := integer(trunc(-2.9));
  constant signum   : real := sign(-3.0);
  constant hi       : real := realmax(3.0, 4.0);
  constant lo       : real := realmin(3.0, 4.0);
  constant quadrant : real := arctan(1.0) * 4.0;
  constant cube     : real := cbrt(27.0);
  constant degrees  : real := math_pi * math_rad_to_deg;
  constant power    : real := 2.0 ** 10.0;
  constant ipower   : real := 2 ** 0.5;
  constant remains  : real := 7.0 mod 3.0;
  constant negrem   : real := (-7.0) mod 3.0;
  constant asinh_ok : boolean := arcsinh(0.0) = 0.0;
  constant acosh_ok : boolean := arccosh(1.0) = 0.0;
  constant atanh_ok : boolean := arctanh(0.0) = 0.0;

  -- Enough of the identities to show the constants are the real numbers
  -- and not approximations of something else.
  constant pi_ok    : boolean := cos(math_pi) = -1.0;
  constant half_ok  : boolean := sin(math_pi_over_2) = 1.0;
  constant root_ok  : boolean := math_sqrt_2 * math_1_over_sqrt_2 > 0.999;
  constant e_ok     : boolean := exp(1.0) > 2.718 and exp(1.0) < 2.719;

end package math_real_static;
