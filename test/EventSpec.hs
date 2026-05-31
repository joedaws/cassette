{-# LANGUAGE OverloadedStrings #-}
module EventSpec (spec) where

import qualified Cassette
import Cassette (cassetteText)
import Event
import Test.Hspec
import Data.Maybe (isJust)

spec :: Spec
spec = do
  describe "initialState" $ do
    it "starts with exactly 1 cassette" $
      length (_cassettes initialState) `shouldBe` 1

  describe "calcMaxCassettes" $ do
    it "returns 1 for a very short terminal" $
      calcMaxCassettes 5 `shouldBe` 1
    it "returns correct value for height 24" $
      -- (24 - 4) `div` 2 = 10
      calcMaxCassettes 24 `shouldBe` 10
    it "returns correct value for height 40" $
      -- (40 - 4) `div` 2 = 18
      calcMaxCassettes 40 `shouldBe` 18

  describe "addCassetteToSt" $ do
    it "adds a cassette below existing ones" $
      length (_cassettes (addCassetteToSt st40)) `shouldBe` 2
    it "sets focusIdx to the new cassette's index" $
      _focusIdx (addCassetteToSt st40) `shouldBe` 1
    it "new cassette is empty" $
      cassetteText (last (_cassettes (addCassetteToSt st40))) `shouldBe` ""
    it "sets statusMsg at max capacity" $
      isJust (_statusMsg (addCassetteToSt st5)) `shouldBe` True
    it "does not add a cassette at max capacity" $
      length (_cassettes (addCassetteToSt st5)) `shouldBe` 1

  describe "focusNextSt" $ do
    it "advances focusIdx from 0 to 1" $
      _focusIdx (focusNextSt (addCassetteToSt st40){ _focusIdx = 0}) `shouldBe` 1
    it "wraps focusIdx from last to 0" $
      _focusIdx (focusNextSt (addCassetteToSt st40)) `shouldBe` 0  -- focusIdx=1, n=2 → 0

  describe "focusPrevSt" $ do
    it "decrements focusIdx from 1 to 0" $
      _focusIdx (focusPrevSt (addCassetteToSt st40)) `shouldBe` 0
    it "wraps focusIdx from 0 to last" $
      _focusIdx (focusPrevSt (addCassetteToSt st40){ _focusIdx = 0}) `shouldBe` 1

  describe "modifyFocusedCassetteSt" $ do
    it "only modifies the cassette at focusIdx" $ do
      let st2  = addCassetteToSt st40  -- focusIdx=1
          st2' = modifyFocusedCassetteSt (\t -> Cassette.insert t 'x') st2
      cassetteText (_cassettes st2' !! 0) `shouldBe` ""
      cassetteText (_cassettes st2' !! 1) `shouldBe` "x"

  describe "tickTimer" $ do
    it "does nothing when no timer" $
      _timerSecs (tickTimer stNoTimer) `shouldBe` Nothing
    it "decrements positive timer" $
      _timerSecs (tickTimer (stWithTimer 60)) `shouldBe` Just 59
    it "holds at zero (no underflow)" $
      _timerSecs (tickTimer (stWithTimer 0)) `shouldBe` Just 0

  describe "advanceReel" $ do
    it "increments reelRotation from 0 to 1" $
      _reelRotation (advanceReel initialState) `shouldBe` 1
    it "wraps reelRotation from 3 to 0" $
      _reelRotation (advanceReel (initialState { _reelRotation = 3 })) `shouldBe` 0

  where
    st40        = initialState { _termHeight = 40 }
    st5         = initialState { _termHeight = 5 }
    stWithTimer n = initialState { _timerSecs = Just n }
    stNoTimer   = initialState { _timerSecs = Nothing }
