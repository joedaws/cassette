module Render
  ( RenderCtx (..),
    Effect,
    renderCassetteText,
    edgeFadeEffect,
  )
where

import Brick.Widgets.Core (hBox, modifyDefAttr, str)
import qualified Brick.Types as T
import qualified Data.Text as DT
import qualified Graphics.Vty as V

-- | Context available to every rendering effect.
data RenderCtx = RenderCtx
  { rcPos :: Int,
    rcLen :: Int,
    rcChar :: Char,
    rcFocused :: Bool
  }

-- | An effect modifies a Vty Attr based on character context.
type Effect = RenderCtx -> V.Attr -> V.Attr

-- | Apply a list of effects in order and render each character.
renderCassetteText :: [Effect] -> Bool -> DT.Text -> T.Widget n
renderCassetteText effects isFocused text =
  hBox
    [ modifyDefAttr (applyEffects ctx) (str [c])
      | (i, c) <- zip [0 ..] (DT.unpack text),
        let ctx = RenderCtx {rcPos = i, rcLen = len, rcChar = c, rcFocused = isFocused}
    ]
  where
    len = DT.length text
    applyEffects ctx attr = foldl (\a e -> e ctx a) attr effects

-- | Fades foreground toward the cassette background color based on distance from center.
-- Also sets the background explicitly so the gradient edge matches the bg exactly.
edgeFadeEffect :: Effect
edgeFadeEffect ctx attr =
  attr
    { V.attrForeColor = V.SetTo (V.linearColor r g b),
      V.attrBackColor = V.SetTo bgColor
    }
  where
    center = rcLen ctx `div` 2
    halfLen = center
    dist = abs (rcPos ctx - center)
    t = max 0.0 (min 1.0 (1.0 - fromIntegral dist / fromIntegral (max 1 halfLen))) :: Double
    (fgRGB, bgRGB)
      | rcFocused ctx = ((0, 0, 0), (170, 170, 0))
      | otherwise = ((255, 255, 255), (0, 0, 170))
    (bgR, bgG, bgB) = bgRGB
    bgColor = V.linearColor bgR bgG bgB
    (r, g, b) = lerpRGB t fgRGB bgRGB

lerpRGB :: Double -> (Int, Int, Int) -> (Int, Int, Int) -> (Int, Int, Int)
lerpRGB t (r1, g1, b1) (r2, g2, b2) =
  (lerp r1 r2, lerp g1 g2, lerp b1 b2)
  where
    lerp a c = round (fromIntegral a * t + fromIntegral c * (1 - t))
