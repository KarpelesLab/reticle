-- V0714: an entity with no architecture cannot be elaborated.
entity error_no_architecture is
  port (
    d : in  bit;
    q : out bit
  );
end entity;
