-- top: error_recursive_hierarchy
-- V0709: an entity that instantiates itself has no finite hierarchy.
entity error_recursive_hierarchy is
  port (
    d : in  bit;
    q : out bit
  );
end entity;

architecture rtl of error_recursive_hierarchy is
begin
  inner : entity work.error_recursive_hierarchy port map (d => d, q => q);
end architecture;
