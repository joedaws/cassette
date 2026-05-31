module CassetteSpec (spec) where

import Control.Lens (view)
import Data.Text as T
import Cassette
import Test.Hspec

-- define some test text
simpleText :: T.Text
simpleText = T.pack "hello it is me"

mkTp :: Int -> Cassette
mkTp = initCassette simpleText

getText :: Cassette -> T.Text
getText t = view leftText t `T.append` view rightText t

spec :: Spec
spec = do
  describe "printCassette" $ do
    it "when left spacing is needed" $ do
      printCassette (mkTp 3) `shouldBe` T.pack "  hel|lo it"
    it "when no spacing is needed" $ do
      printCassette (mkTp 7) `shouldBe` T.pack "llo i|t is "
    it "when right spacing is needed" $ do
      printCassette (mkTp 12) `shouldBe` T.pack "t is |me   "

  describe "insert" $ do
    it "when cursor at the end" $ do
      getText (insert (mkTp 14) '!') `shouldBe` simpleText `T.snoc` '!'
    it "when the cursor is in the middle" $ do
      getText (insert (mkTp 2) '!') `shouldBe` T.pack "he!llo it is me"
    it "when the cursor is at the beginning" $ do
      getText (insert (mkTp 0) '!') `shouldBe` T.pack "!hello it is me"
    it "when two chars are added" $ do
      getText (insert (insert (mkTp 14) '!') '!') `shouldBe` T.pack "hello it is me!!"

  describe "backspace" $ do
    it "when cursor at the end" $ do
      getText (backspace (mkTp 14)) `shouldBe` T.init simpleText
    it "when cursor at the beginning" $ do
      getText (backspace (mkTp 0)) `shouldBe` simpleText

  describe "delete" $ do
    it "when cursor at the end" $ do
      getText (delete (mkTp 14)) `shouldBe` simpleText
    it "when cursor at the beginning" $ do
      getText (delete (mkTp 0)) `shouldBe` T.tail simpleText

  describe "forward" $ do
    it "when cursor at the beginning" $ do
      forward (mkTp 0) `shouldBe` mkTp 1
    it "when cursor at the end" $ do
      forward (mkTp 14) `shouldBe` mkTp 14

  describe "rewind" $ do
    it "when cursor at the beginning" $ do
      rewind (mkTp 0) `shouldBe` mkTp 0
    it "when cursor at the end" $ do
      rewind (mkTp 14) `shouldBe` mkTp 13
