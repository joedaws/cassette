-- | Multiple cassettes in a deck
module Deck (cassetteWidth, cassetteRows) where

-- | Number of rows each cassette widget occupies (separator + text)
cassetteRows :: Int
cassetteRows = 2

-- | Width of the cassette text region (terminal width minus side padding)
cassetteWidth :: Int -> Int
cassetteWidth w = max 20 (w - 2)
