-- The IEEE.MATH_REAL package (IEEE 1076.2-1996, as carried into
-- IEEE 1076-2008).
--
-- Clean-room source for Reticle. This file is an original implementation
-- of the package interface defined by the relevant IEEE standard, written
-- for Reticle; it is not derived from the IEEE source distribution.
--
-- The constants are mathematical values, written out to more digits than
-- an IEEE double can hold so the nearest representable value is reached
-- whatever the rounding. Every function is `attribute foreign` and
-- evaluated natively over Rust's `f64`, so the package has no body.

package math_real is

  constant math_e              : real := 2.71828_18284_59045_23536;
  constant math_1_over_e       : real := 0.36787_94411_71442_32160;
  constant math_pi             : real := 3.14159_26535_89793_23846;
  constant math_2_pi           : real := 6.28318_53071_79586_47693;
  constant math_1_over_pi      : real := 0.31830_98861_83790_67154;
  constant math_pi_over_2      : real := 1.57079_63267_94896_61923;
  constant math_pi_over_3      : real := 1.04719_75511_96597_74615;
  constant math_pi_over_4      : real := 0.78539_81633_97448_30962;
  constant math_3_pi_over_2    : real := 4.71238_89803_84689_85769;
  constant math_log_of_2       : real := 0.69314_71805_59945_30942;
  constant math_log_of_10      : real := 2.30258_50929_94045_68402;
  constant math_log2_of_e      : real := 1.44269_50408_88963_40736;
  constant math_log10_of_e     : real := 0.43429_44819_03251_82765;
  constant math_sqrt_2         : real := 1.41421_35623_73095_04880;
  constant math_1_over_sqrt_2  : real := 0.70710_67811_86547_52440;
  constant math_sqrt_pi        : real := 1.77245_38509_05516_02730;
  constant math_deg_to_rad     : real := 0.01745_32925_19943_29577;
  constant math_rad_to_deg     : real := 57.29577_95130_82320_87680;

  ----------------------------------------------------------------------
  -- Rounding, sign and extrema.
  ----------------------------------------------------------------------

  function sign    (x : real) return real;
  function ceil    (x : real) return real;
  function floor   (x : real) return real;
  function round   (x : real) return real;
  function trunc   (x : real) return real;
  function "mod"   (x, y : real) return real;
  function realmax (x, y : real) return real;
  function realmin (x, y : real) return real;

  ----------------------------------------------------------------------
  -- A uniform pseudo-random value in (0.0, 1.0), advancing both seeds.
  ----------------------------------------------------------------------

  procedure uniform (variable seed1, seed2 : inout positive;
                     variable x : out real);

  ----------------------------------------------------------------------
  -- Roots, powers, exponentials and logarithms.
  ----------------------------------------------------------------------

  function sqrt (x : real) return real;
  function cbrt (x : real) return real;

  function "**" (x : integer; y : real) return real;
  function "**" (x : real; y : real) return real;

  function exp   (x : real) return real;
  function log   (x : real) return real;
  function log2  (x : real) return real;
  function log10 (x : real) return real;
  function log   (x : real; base : real) return real;

  ----------------------------------------------------------------------
  -- Trigonometric and hyperbolic functions. Angles are in radians.
  ----------------------------------------------------------------------

  function sin    (x : real) return real;
  function cos    (x : real) return real;
  function tan    (x : real) return real;
  function arcsin (x : real) return real;
  function arccos (x : real) return real;
  function arctan (y : real) return real;
  function arctan (y : real; x : real) return real;

  function sinh    (x : real) return real;
  function cosh    (x : real) return real;
  function tanh    (x : real) return real;
  function arcsinh (x : real) return real;
  function arccosh (x : real) return real;
  function arctanh (x : real) return real;

  attribute foreign of sign    : function is "reticle: builtin";
  attribute foreign of ceil    : function is "reticle: builtin";
  attribute foreign of floor   : function is "reticle: builtin";
  attribute foreign of round   : function is "reticle: builtin";
  attribute foreign of trunc   : function is "reticle: builtin";
  attribute foreign of "mod"   : function is "reticle: builtin";
  attribute foreign of realmax : function is "reticle: builtin";
  attribute foreign of realmin : function is "reticle: builtin";
  attribute foreign of uniform : procedure is "reticle: builtin";
  attribute foreign of sqrt    : function is "reticle: builtin";
  attribute foreign of cbrt    : function is "reticle: builtin";
  attribute foreign of "**"    : function is "reticle: builtin";
  attribute foreign of exp     : function is "reticle: builtin";
  attribute foreign of log     : function is "reticle: builtin";
  attribute foreign of log2    : function is "reticle: builtin";
  attribute foreign of log10   : function is "reticle: builtin";
  attribute foreign of sin     : function is "reticle: builtin";
  attribute foreign of cos     : function is "reticle: builtin";
  attribute foreign of tan     : function is "reticle: builtin";
  attribute foreign of arcsin  : function is "reticle: builtin";
  attribute foreign of arccos  : function is "reticle: builtin";
  attribute foreign of arctan  : function is "reticle: builtin";
  attribute foreign of sinh    : function is "reticle: builtin";
  attribute foreign of cosh    : function is "reticle: builtin";
  attribute foreign of tanh    : function is "reticle: builtin";
  attribute foreign of arcsinh : function is "reticle: builtin";
  attribute foreign of arccosh : function is "reticle: builtin";
  attribute foreign of arctanh : function is "reticle: builtin";

end package math_real;
