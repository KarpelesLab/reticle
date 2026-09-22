-- Extended identifiers are case-sensitive and may contain any graphic
-- character; a doubled backslash stands for one backslash.
signal \Foo Bar\ : bit;
signal \foo bar\ : bit;
signal \a\\b\ : bit;
signal \entity\ : bit;
signal \123\ : bit;
signal \x!"#$%&'()*+,-./:;<=>?@[]^_`{|}~\ : bit;
signal \Ärger\ : bit;
\a\ <= \b\ ;
