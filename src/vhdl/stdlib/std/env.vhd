-- The STD.ENV package (IEEE 1076-2008 clause 16.5).
--
-- Clean-room source for Reticle. Everything here controls or inspects the
-- simulator, so each subprogram is `attribute foreign` and implemented
-- natively; the package has no body.

package env is

  procedure stop (status : integer);
  procedure stop;

  procedure finish (status : integer);
  procedure finish;

  function resolution_limit return delay_length;

  attribute foreign of stop             : procedure is "reticle: builtin";
  attribute foreign of finish           : procedure is "reticle: builtin";
  attribute foreign of resolution_limit : function  is "reticle: builtin";

end package env;
